//! Read-only checks against this machine's real daemons. They change
//! nothing; run them with `cargo test -p aizome-sys --test live -- --ignored
//! --nocapture` on a machine that has the services.

mod common;

use aizome_sys::Wake;
use common::eventually;

#[test]
#[ignore = "reads the real system bus"]
fn network_reads_networkmanager() {
    let network = aizome_sys::network::Network::new(Wake::none());
    assert!(eventually(|| network.available()), "NetworkManager is not running");
    let state = network.snapshot();
    println!("NetworkManager {} state {:?} connectivity {:?}", state.version, state.state, state.connectivity);
    for device in &state.devices {
        println!("  device {} {:?} {:?} {}", device.interface, device.kind, device.state, device.hw_address);
    }
    for network in &state.networks {
        println!(
            "  network {:?} {}% {:?} {:?} known={} connected={}",
            network.ssid, network.strength, network.security, network.bands, network.known, network.connected
        );
    }
    for connection in &state.connections {
        println!("  saved {:?} {:?} active={} autoconnect={}", connection.id, connection.kind, connection.active, connection.autoconnect);
    }
    if let Some(details) = state.primary_details() {
        println!("  primary: {details:?}");
        assert!(!details.interface.is_empty());
    }
    println!("  radios: {:?} airplane={}", state.radios, state.airplane_mode);
    assert!(!state.devices.is_empty());
}

#[test]
#[ignore = "reads the real system bus"]
fn bluetooth_reads_bluez() {
    let bt = aizome_sys::bluetooth::Bluetooth::new(Wake::none());
    assert!(eventually(|| bt.available()), "BlueZ is not running");
    let state = bt.snapshot();
    for adapter in &state.adapters {
        println!(
            "adapter {} {} powered={} ({}) discoverable={} discovering={}",
            adapter.path, adapter.alias, adapter.powered, adapter.power_state, adapter.discoverable, adapter.discovering
        );
    }
    for device in &state.devices {
        println!(
            "  device {} {:?} {:?} paired={} connected={} battery={:?}",
            device.address, device.name, device.kind, device.paired, device.connected, device.battery
        );
    }
    assert!(!state.adapters.is_empty());
}

#[test]
#[ignore = "reads the real sound server"]
fn audio_reads_the_sound_server() {
    let audio = aizome_sys::audio::Audio::new(Wake::none());
    assert!(eventually(|| audio.available() && !audio.snapshot().sinks.is_empty()), "no sound server");
    let state = audio.snapshot();
    println!("server: {}", state.server);
    for sink in &state.sinks {
        println!(
            "  output {} {:?} {:.0}% muted={} default={} icon={} ports={:?}",
            sink.index, sink.description, sink.volume * 100.0, sink.muted, sink.is_default, sink.icon_name,
            sink.ports.iter().map(|p| (&p.name, p.available)).collect::<Vec<_>>()
        );
    }
    for source in &state.sources {
        println!("  input {} {:?} {:.0}% muted={} default={}", source.index, source.description, source.volume * 100.0, source.muted, source.is_default);
    }
    for stream in state.playback.iter().chain(&state.recording) {
        println!("  stream {} {:?} ({}) {:.0}% on {}", stream.index, stream.application, stream.app_id, stream.volume * 100.0, stream.device);
    }
    assert!(state.default_output().is_some());
}
