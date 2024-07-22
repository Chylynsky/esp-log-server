***ESP32 Log Server***

ESP32 log server created using *Embassy* framework. 
Serves as an utility app for reading logs from another device via UART and seding them to the TCP connected client.

Below is a part of the *[env]* section in *config.toml*, WIFI_SSID and WIFI_PASS must be both set.
```
[env]
WIFI_SSID = "Your Wifi SSID"
WIFI_PASS = "Your password"
```

Bear in mind that those variables may be also supplied via command line while building.
```
WIFI_SSID="ssid" WIFI_PASS="pass" cargo espflash flash --release
cargo espflash monitor
```

Default server port is 3030, optionally overriden with LOG_SERVER_PORT environment variable as shown above.
Logs can be then read with *nc*:
```
nc <IP> 3030
```
