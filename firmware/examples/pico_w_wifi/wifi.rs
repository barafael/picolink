//! Scanning, continuously, and streaming each scan to the host.

use cyw43::{BssInfo, Control, ScanOptions};
use defmt::{debug, info, trace, warn};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer, with_timeout};
use heapless::Vec;
use picolink_firmware::link::Tx;
use picolink_protocol::wifi::{MAX_NETWORKS, Network, SCAN_TIMEOUT_MS, SSID_LEN, ScanEvent, ScanTopic, Ssid};
use postcard_rpc::server::Sender;

/// The Wi-Fi chip, shared by the scanner and the endpoints. Everything runs on one executor, hence `NoopRawMutex`.
pub type Wifi = Mutex<NoopRawMutex, Control<'static>>;

/// Pause between the end of one scan and the start of the next. A scan itself takes about 2 s.
const SCAN_PAUSE: Duration = Duration::from_secs(3);

/// The "privacy" bit of the 802.11 capability field: the network is encrypted.
const CAPABILITY_PRIVACY: u16 = 1 << 4;

type Networks = Vec<Network, MAX_NETWORKS>;

/// Scans forever, and publishes every scan, whether a host listens or not.
#[embassy_executor::task]
pub async fn scan_continuously(wifi: &'static Wifi, sender: Sender<Tx>) -> ! {
    let mut seq = 0u32;
    for scan in (0..=u16::MAX).cycle() {
        // Released before publishing, so that joining doesn't wait for a slow host.
        let networks = scan_once(&mut *wifi.lock().await).await;
        let count = networks.len() as u8;
        for network in networks {
            publish(&sender, &mut seq, &ScanEvent::Network { scan, network }).await;
        }
        publish(&sender, &mut seq, &ScanEvent::Done { scan, networks: count }).await;
        Timer::after(SCAN_PAUSE).await;
    }
    unreachable!("the scan counter cycles")
}

async fn publish(sender: &Sender<Tx>, seq: &mut u32, event: &ScanEvent) {
    // Fails while no host is listening, then the event is just dropped.
    if sender.publish::<ScanTopic>((*seq).into(), event).await.is_err() {
        trace!("Dropped scan event");
    }
    *seq = seq.wrapping_add(1);
}

/// The networks in range, strongest first.
async fn scan_once(control: &mut Control<'static>) -> Networks {
    let mut networks = Networks::new();
    let scanning = async {
        let mut scanner = control.scan(ScanOptions::default()).await;
        while let Some(bss) = scanner.next().await {
            add(&mut networks, &bss);
        }
    };
    if with_timeout(Duration::from_millis(SCAN_TIMEOUT_MS), scanning).await.is_err() {
        warn!("Scan timed out, reporting what it found so far");
    }
    networks.sort_unstable_by_key(|network| -network.rssi);
    info!("Found {} networks", networks.len());
    networks
}

/// Merges a scan report into `networks`: a scan reports each network several times, keep the strongest.
fn add(networks: &mut Networks, bss: &BssInfo) {
    // Copied out first: `BssInfo` is packed, so its fields can't be borrowed.
    let (ssid, ssid_len, bssid, chanspec, rssi, capability) = (bss.ssid, bss.ssid_len, bss.bssid, bss.chanspec, bss.rssi, bss.capability);
    let Some(ssid) = ssid
        .get(..usize::from(ssid_len).min(SSID_LEN))
        .and_then(|ssid| core::str::from_utf8(ssid).ok())
    else {
        debug!("Skipping a network whose SSID is not UTF-8");
        return;
    };
    let network = Network {
        // At most SSID_LEN bytes, so it fits.
        ssid: Ssid::try_from(ssid).unwrap_or_default(),
        bssid,
        // The low byte of the channel specification is the channel number.
        channel: (chanspec & 0xFF) as u8,
        rssi,
        secured: capability & CAPABILITY_PRIVACY != 0,
    };

    if let Some(known) = networks.iter_mut().find(|known| known.bssid == network.bssid) {
        if network.rssi > known.rssi {
            *known = network;
        }
    } else if let Err(network) = networks.push(network) {
        // Full: keep the strongest networks.
        if let Some(weakest) = networks.iter_mut().min_by_key(|known| known.rssi)
            && network.rssi > weakest.rssi
        {
            *weakest = network;
        }
    }
}
