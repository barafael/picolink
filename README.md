# picolink

A minimal end-to-end stack: RP2040 firmware with one sensor and one actuator, and two alternative ways to control it:
a server that owns the USB connection, with a UI for the browser or the desktop,
or a web page that talks to the device directly through WebUSB.

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

## Layout

- `protocol/`: the [postcard-rpc](https://github.com/jamesmunns/postcard-rpc) interface, `no_std`, shared by everything else.
  - Endpoints: `SerialNumberEndpoint`, `GetActuatorEndpoint`, `SetActuatorEndpoint`.
  - Topic: `ReadingTopic`, sent from the device to the host.
- `firmware/`: embassy on both cores.
  - Core 0 runs the application: `sensor.rs` publishes readings, `actuator.rs` reacts to commands.
  - Core 1 runs the USB device, the postcard-rpc server (`rpc.rs`), and a task that publishes readings.
  - The cores only talk through the `INCOMING` and `OUTGOING` pub-sub channels in `main.rs`.
    Their payloads in `messages.rs` are internal and not part of the wire protocol.
  - The actuator endpoints are answered by core 0: the handler publishes a `Command` and waits for the actuator's `Event`.
- `mock-firmware/`: a mock of the firmware, for running without hardware.
  - It serves the same protocol with the real postcard-rpc server, over in-memory channels instead of USB.
  - Its test exercises every endpoint and the topic.
- `app/`: a Dioxus fullstack app.
  - `server` feature: `device.rs` connects to the device or the mock firmware and publishes its `DeviceState`.
    The `Device` actor takes `Command`s over a channel and stops once no sender is left. `main.rs` spawns it.
  - `api.rs`: the `/api/device` websocket. It streams `DeviceState` to UIs and forwards their `Command`s to the device.
  - `web` / `desktop` features: `ui.rs`, the same UI for both.
- `webusb-app/`: a Leptos page, the alternative to `app/` without a server.
  - `usb.rs`: finds the device through WebUSB and connects postcard-rpc's WebUSB client to it.
  - `main.rs`: the same UI as `app/`, styled by the same `app/assets/main.css`.

postcard-rpc identifies endpoints and topics by a hash of path and schema.
So a host and a device built from different protocol versions reject each other's messages instead of misreading them.

The USB VID:PID is shared with the MT21. The server and the WebUSB app only claim devices whose USB product string is `picolink`.

## Running

The toolchain is pinned to stable (`rust-toolchain.toml`). The reason is the current nightly, which hits a compiler bug in `postcard-schema`.
postcard-rpc is a git dependency pinned to a commit, because no release supports embassy-usb 0.6 yet.

### Firmware

```sh
cd firmware
cargo run --release
```

This uses `probe-rs` with a debug probe and shows defmt logs.
Without a probe, switch the runner in `firmware/.cargo/config.toml` to `picotool`, then hold BOOTSEL while plugging in the Pico.

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

Hosted at <https://barafael.github.io/picolink/>, deployed from `main` by CI.

To run it locally, install [Trunk](https://trunkrs.dev).

```sh
cd webusb-app
trunk serve --open   # http://127.0.0.1:8080
```

- WebUSB needs a Chromium-based browser and a secure origin (`localhost` or https).
- Only one program can claim the device at a time, so stop the server first. Both default to port 8080 as well.
- It only builds for wasm32, so the workspace's `default-members` leave it out of a plain `cargo build` or `cargo test`.
  Run cargo from `webusb-app/` with `--target wasm32-unknown-unknown`: its `.cargo/config.toml` enables web-sys's WebUSB API.
- The mock firmware running as a USB device (below) should work too: like the firmware, it has a vendor-specific interface.

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
cargo test -p picolink-mock-firmware
cargo test -p picolink-app --no-default-features --features server   # the device actor against the mock firmware
```
