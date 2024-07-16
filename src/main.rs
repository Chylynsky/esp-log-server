#![no_std]
#![no_main]


use embassy_executor::Spawner;

use embassy_net::{
    tcp::TcpSocket, Config, IpListenEndpoint, Stack, StackResources,
};
use embassy_time::{Duration, Timer};

use esp_backtrace as _;
use esp_hal::{
    clock::ClockControl, peripherals::Peripherals, prelude::*, rng::Rng, system::SystemControl,
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};

macro_rules! make_static {
    ($t:path,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASS");

const RX_BUFFER_SIZE: usize = 512;
const TX_BUFFER_SIZE: usize = 512;

async fn wifi_connect(
    spawner: Spawner,
    wifi_interface: WifiDevice<'static, WifiStaDevice>,
    wifi_ctrl: WifiController<'static>,
) -> &'static Stack<WifiDevice<'static, WifiStaDevice>> {
    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*make_static!(
        Stack<WifiDevice<'_, WifiStaDevice>>,
        Stack::new(
            wifi_interface,
            Config::dhcpv4(Default::default()),
            make_static!(StackResources<3>, StackResources::<3>::new()),
            seed,
        )
    );

    spawner.spawn(connection(wifi_ctrl)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    stack
}

#[main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let peripherals = Peripherals::take();

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    let timer = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG1, &clocks, None).timer0;

    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, wifi_ctrl) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let timer_group0 = TimerGroup::new_async(peripherals.TIMG0, &clocks);
    esp_hal_embassy::init(&clocks, timer_group0);

    // static buffers to not need a huge task-arena
    let mut RX_BUFFER: [u8; RX_BUFFER_SIZE] = [0; RX_BUFFER_SIZE];
    let mut TX_BUFFER: [u8; TX_BUFFER_SIZE] = [0; TX_BUFFER_SIZE];

    let wifi_stack = wifi_connect(spawner, wifi_interface, wifi_ctrl).await;

    let ipv4_cfg = wifi_stack
        .config_v4()
        .expect("Unable to acquire IPv4 address.");
    let mut socket = TcpSocket::new(wifi_stack, &mut RX_BUFFER, &mut TX_BUFFER);

    loop {
        let local_ep = IpListenEndpoint {
            addr: Some(ipv4_cfg.address.address().into_address()),
            port: 3030,
        };

        socket.accept(local_ep).await.unwrap();
        socket.write("dupa".as_bytes()).await.unwrap();

        Timer::after(Duration::from_millis(10000)).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
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
                ssid: WIFI_SSID.try_into().unwrap(),
                password: WIFI_PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
