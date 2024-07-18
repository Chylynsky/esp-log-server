***ESP32 Log Server***

ESP32 log server created using *Embassy* framework. 
Serves as an utility app for reading logs from another device via UART and seding them to the TCP connected client.

Below is the config.toml that must be created in order to build properly and setup environment variables (ssid, password). Put it in in .cargo/config.toml.
```
[target.xtensa-esp32-none-elf]
runner = "espflash flash --monitor"

[env]
ESP_LOGLEVEL = "INFO"
WIFI_SSID = "Your Wifi SSID"
WIFI_PASS = "Your password"
LOG_SERVER_PORT = "3030"
WIFI_STACK_SEED = "1234"

[build]
rustflags = ["-C", "link-arg=-nostartfiles"]

target = "xtensa-esp32-none-elf"

[unstable]
build-std = ["core"]

```

Default server port is 3030, optionally overriden with LOG_SERVER_PORT environment variable as shown above.
Logs can be then read with *nc*:
```
nc <IP> 3030
```
