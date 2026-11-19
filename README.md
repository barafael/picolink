# picolink

A minimal end-to-end stack for talking to an RP2040 over USB with [postcard-rpc](https://github.com/jamesmunns/postcard-rpc):
libraries for the firmware and for the host, and example firmwares built from them.

The main example is a firmware with one sensor and one actuator, and two alternative ways to control it:
a server that owns the USB connection, with a UI for the browser or the desktop,
or a web page that talks to the device directly through WebUSB.
A second example turns a Pico W into a Wi-Fi scanner: a web page lists the networks it sees, live, and joins one through it.

```
┌──────────── RP2040 ────────────┐              ┌──────── host ────────┐             ┌───────────────┐
│ core 0: sensor, actuator       │   USB bulk   │ server (axum)        │  websocket  │ web UI        │
│   ▲ INCOMING      OUTGOING ▼   │◄────────────►│  device task         │◄───────────►│ or desktop UI │
│ core 1: USB, postcard-rpc      │ postcard-rpc │  /api/device         │    JSON     │ (Dioxus)      │
└────────────────────────────────┘              └──────────────────────┘             └───────────────┘
                ▲
                │ or, without a server: USB bulk through WebUSB, postcard-rpc
                ▼
┌──────────── browser ───────────┐
│ WebUSB app (Leptos)            │
└────────────────────────────────┘
```

On a plain Raspberry Pi Pico (not the Pico W), no extra hardware is needed:
the actuator is the onboard LED (GPIO 25), the sensor is the RP2040's internal temperature sensor.
On a Pico W, the onboard LED is wired to the Wi-Fi chip, so it stays dark.

## Layout

- `protocol/`: the postcard-rpc interfaces, `no_std`, shared by everything else.
  - `SerialNumberEndpoint`, served by every firmware, and the USB VID and PID.
  - `sensor_actuator`: `GetActuatorEndpoint`, `SetActuatorEndpoint`, and `ReadingTopic`, sent from the device to the host.
  - `wifi`: `ScanEndpoint`, `JoinEndpoint`, `LeaveEndpoint`.
  - Each module has its firmware's USB product string, `picolink` and `picolink-wifi`. Hosts use it to find a device they understand.
- `firmware/`: a library for embassy on the RP2040, and the example firmwares.
  - `link`: the USB device and a postcard-rpc server for the firmware's own endpoints, defined with `define_dispatch!`.
    The USB serial number string is the flash unique ID in hex, the same number `SerialNumberEndpoint` answers.
    Without one, browsers would not remember WebUSB permissions across replugs.
  - `serial_number`, `multicore` (an executor on core 1) and `temperature` (the RP2040's internal sensor).
  - `examples/sensor_actuator/`: for a plain Pico, on both cores.
    - Core 0 runs the application: `sensor.rs` publishes readings, `actuator.rs` reacts to commands.
    - Core 1 runs the USB device, the postcard-rpc server (`rpc.rs`), and a task that publishes readings.
    - The cores only talk through the `INCOMING` and `OUTGOING` pub-sub channels in `main.rs`.
      Their payloads in `messages.rs` are internal and not part of the wire protocol.
    - The actuator endpoints are answered by core 0: the handler publishes a `Command` and waits for the actuator's `Event`.
  - `examples/pico_w_wifi/`: for a Pico W, on one core.
    `wifi.rs` scans continuously and streams each scan on a topic. The endpoints in `rpc.rs` join and leave.
    Both share the Wi-Fi chip through a mutex: the chip's driver, the USB device and the server share an executor.
    The chip's firmware is vendored in `cyw43-firmware/`.
- `host/`: a library for finding a device by its USB product string (and serial number), connecting to it,
  and making requests that time out.
  - `examples/sensor_actuator.rs`: a command-line host for the `sensor_actuator` firmware.
- `mock-firmware/`: a mock of the `sensor_actuator` firmware, for running without hardware.
  - It serves the same protocol with the real postcard-rpc server, over in-memory channels instead of USB.
  - Its test exercises every endpoint and the topic.
- `app/`: a Dioxus fullstack app for the `sensor_actuator` firmware.
  - `server` feature: `device.rs` connects to the device through `host/`, or to the mock firmware, and publishes its `DeviceState`.
    The `Device` actor takes `Command`s over a channel and stops once no sender is left. `main.rs` spawns it.
  - `api.rs`: the `/api/device` websocket. It streams `DeviceState` to UIs and forwards their `Command`s to the device.
  - `web` / `desktop` features: `ui.rs`, the same UI for both.
- `webusb-app/`: a library for Leptos pages that talk to a device through WebUSB, without a server, and a page per firmware.
  - `usb.rs`: finds a device by its USB product string and connects postcard-rpc's WebUSB client to it.
  - `connection.rs`: keeps a page connected, reconnecting when a device it may use is plugged in again.
  - `src/bin/sensor_actuator.rs` (`index.html`): the same UI as `app/`, styled by the same `app/assets/main.css`.
  - `src/bin/wifi.rs` (`wifi.html`): the networks the Pico W sees, updated with every scan, and a form to join one.

postcard-rpc identifies endpoints and topics by a hash of path and schema.
So a host and a device built from different protocol versions reject each other's messages instead of misreading them.
A test in `protocol/` pins the `sensor_actuator` keys, which devices in the field and the hosted WebUSB app rely on.

The USB VID:PID is shared with the MT21. The server and the WebUSB app only claim devices whose USB product string is `picolink`.

## Running

The toolchain is pinned to stable (`rust-toolchain.toml`). The reason is the current nightly, which hits a compiler bug in `postcard-schema`.
postcard-rpc is a git dependency pinned to a commit, because no release supports embassy-usb 0.6 yet.

### Firmware

```sh
cd firmware
cargo run --release --example sensor_actuator   # plain Pico
cargo run --release --example pico_w_wifi       # Pico W, see below
```

This uses `probe-rs` with a debug probe and shows defmt logs.
Without a probe, switch the runner in `firmware/.cargo/config.toml` to `picotool`, then hold BOOTSEL while plugging in the Pico.

The `sensor_actuator` firmware can also be driven from the command line, without the server:

```sh
cargo run -p picolink-host --example sensor_actuator -- info
cargo run -p picolink-host --example sensor_actuator -- led toggle
cargo run -p picolink-host --example sensor_actuator -- watch 10
```

### Pico W: Wi-Fi

```sh
cd firmware && cargo run --release --example pico_w_wifi   # in one terminal, for the logs
cd webusb-app && trunk serve wifi.html --open              # in another: the page, see WebUSB app below
```

Or use the hosted page, <https://barafael.github.io/picolink/wifi/>.

- The Pico W scans about every 5 seconds and reports up to 20 networks, the strongest ones. The page shows complete scans only.
- Click a network to put it into the join form, or type the name of a hidden one.
  Leave the passphrase empty for open networks. WPA2 and WPA3 work, with passphrases of 8 to 63 characters.
- Joining waits for a running scan, and gives up after 20 seconds.
- Joining only associates with the network. There is no IP stack, so the Pico W gets no address.
- The image includes the Wi-Fi chip's firmware, about 230 KiB, so flashing takes longer than for the other example.

### Server and UI

Requires the [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started) 0.7.

```sh
cd app
dx serve --web -p picolink-app                            # server + browser UI on http://127.0.0.1:8080
dx serve --desktop -p picolink-app                        # server + desktop window
PICOLINK_MOCK_FIRMWARE=1 dx serve --web -p picolink-app   # without hardware, using mock-firmware/
```

Both commands build and start the server along with the client.
The desktop UI is a client of the server, just like the browser. By default, both use `127.0.0.1:8080`.
For release builds, `dx bundle --web --release -p picolink-app` produces the `server` binary next to its `public/` directory.
Use the `IP` and `PORT` environment variables to choose where the server listens.

Always pass `-p picolink-app`. Without it, dx 0.7 looks up the workspace's `default-members` relative to the current directory
instead of the workspace root, and panics with "No such file or directory" when run from `app/`.

### WebUSB app

Hosted at <https://barafael.github.io/picolink/> and, for the Pico W, <https://barafael.github.io/picolink/wifi/>.
Deployed from `main` by CI.

To run them locally, install [Trunk](https://trunkrs.dev).

```sh
cd webusb-app
trunk serve --open             # sensor_actuator, http://127.0.0.1:8080
trunk serve wifi.html --open   # pico_w_wifi
```

- WebUSB needs a Chromium-based browser and a secure origin (`localhost` or https).
- Only one program can claim the device at a time, so stop the server first. Both default to port 8080 as well.
- It only builds for wasm32, so the workspace's `default-members` leave it out of a plain `cargo build` or `cargo test`.
  Run cargo from `webusb-app/` with `--target wasm32-unknown-unknown`: its `.cargo/config.toml` enables web-sys's WebUSB API.
- The mock firmware running as a USB device (below) should work too: like the firmware, it has a vendor-specific interface.
- postcard-rpc's WebUSB client reads one USB packet per transfer, so every message the device sends must fit in 64 bytes.
  That's why the Pico W sends a scan one network at a time. A test in `protocol/` checks the sizes.
- Browsers only remember the permission to use a device that has a USB serial number. Both firmwares share the serial
  number and VID:PID of the board, so granting it once covers both pages.

### Mock firmware as a real USB device (Linux)

With the kernel's `dummy_hcd`, the mock firmware becomes a USB device on this machine.
The server then finds and claims it over nusb like the real firmware, so the host's USB path is exercised too.
The privileged steps need root, everything else runs as your user.

```sh
# Once: let your user open the device.
sudo cp udev/70-picolink.rules /etc/udev/rules.d/ && sudo udevadm control --reload

# Terminal 1: build, load dummy_hcd, and run the mock firmware as root. Stop with Ctrl-C.
./mock-firmware/run-usb-gadget.sh
#   prints: Mock firmware is a USB device on dummy_udc.0

# Terminal 2: check it, then start the server without PICOLINK_MOCK_FIRMWARE.
lsusb -d 16d0:12ce                         # Bus 00X Device 00Y: ID 16d0:12ce picolink picolink
cd app && dx serve --web -p picolink-app   # logs: Connected connection=USB bus …, then Device identified serial_number=4d4f434b00000001
```

Stopping the mock with Ctrl-C works like unplugging the device. `sudo modprobe -r dummy_hcd` unloads the virtual controller afterwards.

### USB access

- Linux: install the udev rule so that the logged-in user, and so the server or the browser, may open the device:
  `sudo cp udev/70-picolink.rules /etc/udev/rules.d/ && sudo udevadm control --reload`, then replug.
- Windows: the firmware declares itself WinUSB-compatible, so no driver installation is needed.

## Editor

The firmware, the host programs and the WebUSB app are built for different targets, so rust-analyzer needs a different setup for each.
In VS Code, open the matching workspace file:

- `picolink-firmware.code-workspace`: `firmware/` for thumbv6m.
- `picolink-host-programs.code-workspace`: the workspace for the host, without the WebUSB app.
- `picolink-webusb-app.code-workspace`: the WebUSB app for wasm32.

## Tests

```sh
cargo test -p picolink-protocol -p picolink-host -p picolink-mock-firmware
cargo test -p picolink-app --no-default-features --features server   # the device actor against the mock firmware
```

The firmware has no tests that run on the host: `cargo clippy --lib --examples` and `cargo build --release --examples` from `firmware/`.
