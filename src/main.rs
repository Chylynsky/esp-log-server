#![no_std]
#![no_main]
#[macro_use]

mod actor;

use actor::{Actor, MessageReceiverFor, MessageSenderFor};
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Config, IpListenEndpoint, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    clock::ClockControl,
    gpio::Io,
    peripherals::{Peripherals, UART0},
    prelude::*,
    rng::Rng,
    system::SystemControl,
    timer::{timg::TimerGroup, ErasedTimer, OneShotTimer, PeriodicTimer},
    uart::Uart,
    Async,
};
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};
use heapless::Vec;

macro_rules! make_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASS");
const SERVER_PORT: &str = env!("LOG_SERVER_PORT");

const DEFAULT_SERVER_PORT: u16 = 3030;
const UART_READ_CHUNK_SIZE: usize = 32;

struct LogSenderMessage {
    chunk: Vec<u8, UART_READ_CHUNK_SIZE>,
}

struct LogSender {
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
}

impl LogSender {
    pub async fn new(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) -> Self {
        Self { stack }
    }
}

impl Actor for LogSender {
    type Message = LogSenderMessage;

    async fn run(&mut self, msg_queue: MessageReceiverFor<Self>) {
        const RX_BUFFER_SIZE: usize = 512;
        const TX_BUFFER_SIZE: usize = 512;

        log::info!("LogSender starting.");

        let mut rx_buffer: [u8; RX_BUFFER_SIZE] = [0; RX_BUFFER_SIZE];
        let mut tx_buffer: [u8; TX_BUFFER_SIZE] = [0; TX_BUFFER_SIZE];

        let addr = self
            .stack
            .config_v4()
            .map(|cfg| cfg.address.address().into_address())
            .expect("Unable to acquire IPv4 address.");
        let port = SERVER_PORT.parse::<u16>().unwrap_or(DEFAULT_SERVER_PORT);

        let mut socket = TcpSocket::new(self.stack, &mut rx_buffer, &mut tx_buffer);

        log::info!("Starting log server at {addr}:{port}");

        socket
            .accept(IpListenEndpoint {
                addr: Some(addr),
                port: SERVER_PORT.parse::<u16>().unwrap_or(DEFAULT_SERVER_PORT),
            })
            .await
            .expect("TcpSocket accept error.");

        loop {
            let msg = msg_queue.receive().await;
            socket
                .write(&msg.chunk)
                .await
                .expect("TcpSocket write failed.");
        }
    }
}

struct UartReaderMessage {}

struct UartReader {
    uart: Uart<'static, UART0, Async>,
    sink: MessageSenderFor<LogSender>,
}

impl Actor for UartReader {
    type Message = UartReaderMessage;

    async fn run(&mut self, _: MessageReceiverFor<Self>) {
        log::info!("UartReader starting.");

        let mut buf = [0u8; UART_READ_CHUNK_SIZE];

        loop {
            let rcv = self
                .uart
                .read_async(&mut buf)
                .await
                .ok()
                .and_then(|rcv_len| buf.get(..rcv_len));

            if rcv.is_none() {
                log::error!("UART read failed.");
                continue;
            }

            self.sink
                .send(LogSenderMessage {
                    chunk: Vec::from_slice(rcv.unwrap())
                        .expect("Invalid slice size for Vec initialization."),
                })
                .await;
        }
    }
}

async fn wifi_connect(
    spawner: Spawner,
    wifi_interface: WifiDevice<'static, WifiStaDevice>,
    wifi_ctrl: WifiController<'static>,
) -> &'static Stack<WifiDevice<'static, WifiStaDevice>> {
    // Init network stack
    let stack = &*make_static!(
        Stack<WifiDevice<'_, WifiStaDevice>>,
        Stack::new(
            wifi_interface,
            Config::dhcpv4(Default::default()),
            make_static!(StackResources<3>, StackResources::<3>::new()),
            str::parse::<u64>(env!("WIFI_STACK_SEED")).expect("Invalid WiFi stack seed."),
        )
    );

    spawner.spawn(connection_task(wifi_ctrl)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    log::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            log::debug!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    stack
}

#[main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    log::info!("Application startup.");
    log::info!("Initializing hardware resources.");

    let peripherals = Peripherals::take();

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks, None);
    let timer_group1 = TimerGroup::new(peripherals.TIMG1, &clocks, None);

    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

    let init = initialize(
        EspWifiInitFor::Wifi,
        PeriodicTimer::new(timer_group0.timer0.into()),
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
        &clocks,
    )
    .expect("EspWifi initialization failed.");

    let wifi = peripherals.WIFI;
    let (wifi_interface, wifi_ctrl) = esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice)
        .expect("WifiStaDevice initialization error.");

    log::info!("Initializing embassy executor.");
    esp_hal_embassy::init(
        &clocks,
        make_static!(
            [OneShotTimer<ErasedTimer>; 1],
            [OneShotTimer::<ErasedTimer>::new(timer_group1.timer0.into())]
        ),
    );

    log::info!("Initializing WiFi.");
    let wifi_stack = wifi_connect(spawner, wifi_interface, wifi_ctrl).await;

    log::info!("Spawning log_sender actor.");
    let log_sender = actor_spawn!(
        spawner,
        log_sender_task,
        LogSender,
        LogSender::new(wifi_stack).await
    );

    log::info!("Spawning uart_sender actor.");
    let _uart_reader = actor_spawn!(
        spawner,
        uart_reader_task,
        UartReader,
        UartReader {
            uart: Uart::new_async(peripherals.UART0, &clocks, io.pins.gpio1, io.pins.gpio3)
                .expect("Failed to initialize UART0, pins=[1, 3]"),
            sink: log_sender,
        }
    );

    log::info!("Application initialization finished.");
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    log::info!("Starting connection task");
    log::debug!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: WIFI_SSID
                    .try_into()
                    .expect("Failed to convert WIFI_SSID to String<32>."),
                password: WIFI_PASSWORD
                    .try_into()
                    .expect("Failed to convert WIFI_PASSWORD to String<64>."),
                ..Default::default()
            });
            controller
                .set_configuration(&client_config)
                .expect("Failed to set WifiController configuration.");
            log::info!("Starting wifi");
            controller
                .start()
                .await
                .expect("Failed to start WifiController.");
            log::debug!("Wifi started.");
        }

        log::info!("About to connect...");

        match controller.connect().await {
            Ok(_) => log::debug!("Wifi connected."),
            Err(e) => {
                log::error!("Failed to connect to Wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    log::info!("Starting net task");
    stack.run().await
}
