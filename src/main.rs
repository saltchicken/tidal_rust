use midir::{MidiOutput, MidiOutputConnection};
use midir::os::unix::VirtualOutput;
use rosc::{OscPacket, OscType, OscTime};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use tokio::net::UdpSocket;
use tokio::time::sleep;

// We wrap the connection and our lease-tracker in a single state struct
struct MidiState {
    conn: MidiOutputConnection,
    note_leases: HashMap<(u8, u8), u64>, // Maps (Channel, Note) to the latest Event ID
    event_counter: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let midi_out = MidiOutput::new("Tidal Rust Client")?;
    let conn_out = midi_out.create_virtual("Tidal Midi Out")?;
    
    // Initialize our shared state
    let midi_tx = Arc::new(Mutex::new(MidiState {
        conn: conn_out,
        note_leases: HashMap::new(),
        event_counter: 0,
    }));

    let addr = "127.0.0.1:3819";
    let socket = UdpSocket::bind(addr).await?;
    println!("🚀 Rust Tidal server listening for OSC on {}", addr);
    println!("🎹 Created virtual ALSA MIDI port: 'Tidal Midi Out'");

    let mut buf = [0u8; 8192];

    loop {
        let (len, _) = socket.recv_from(&mut buf).await?;
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..len]) {
            handle_packet(packet, midi_tx.clone(), None);
        }
    }
}

fn handle_packet(packet: OscPacket, midi_tx: Arc<Mutex<MidiState>>, time: Option<OscTime>) {
    match packet {
        OscPacket::Message(msg) => {
            tokio::spawn(process_message_at_time(msg, midi_tx, time));
        }
        OscPacket::Bundle(bundle) => {
            for element in bundle.content {
                handle_packet(element, midi_tx.clone(), Some(bundle.timetag));
            }
        }
    }
}

async fn process_message_at_time(msg: rosc::OscMessage, midi_tx: Arc<Mutex<MidiState>>, time: Option<OscTime>) {
    // 1. TIMING: Wait until Tidal's scheduled time
    if let Some(t) = time {
        let delay = get_delay(t);
        if delay > Duration::ZERO {
            sleep(delay).await;
        }
    }

    let mut map = HashMap::new();
    let mut i = 0;
    while i < msg.args.len().saturating_sub(1) {
        if let OscType::String(key) = &msg.args[i] {
            map.insert(key.clone(), msg.args[i + 1].clone());
        }
        i += 2;
    }

    let s = map.get("s").and_then(|v| v.clone().string());
    if s.as_deref() != Some("rust") { return; }

    let note = if let Some(mn) = map.get("midinote").and_then(osc_to_f32) {
        mn as u8
    } else if let Some(n_val) = map.get("note").or_else(|| map.get("n")).and_then(osc_to_f32) {
        (n_val + 60.0).clamp(0.0, 127.0) as u8
    } else {
        60
    };

    let chan = map.get("midichan").and_then(osc_to_f32).map(|c| c as u8).unwrap_or(0);
    let amp = map.get("amp").and_then(osc_to_f32).unwrap_or(0.8);
    let velocity = (amp * 127.0).clamp(0.0, 127.0) as u8;
    let duration_sec = map.get("sustain").and_then(osc_to_f32).unwrap_or(0.5);

    let chan = chan.clamp(0, 15);
    let note = note.clamp(0, 127);

    // 2. PLAYING: Lock state, get a unique ID, register our lease, play note
    let event_id = {
        if let Ok(mut state) = midi_tx.lock() {
            state.event_counter += 1;
            let id = state.event_counter;
            
            // Register that THIS specific task now owns the Note Off rights for this pitch
            state.note_leases.insert((chan, note), id);
            
            // Preemptive kill (resets synth envelope)
            let _ = state.conn.send(&[0x80 | chan, note, 0]);
            // Fire new note
            let _ = state.conn.send(&[0x90 | chan, note, velocity]);
            
            id
        } else {
            return;
        }
    };

    // 3. STOPPING: Wait for sustain length
    sleep(Duration::from_secs_f32(duration_sec)).await;
    
    // 4. CLEANUP: Only send Note Off if another task hasn't stolen our lease
    if let Ok(mut state) = midi_tx.lock() {
        if state.note_leases.get(&(chan, note)) == Some(&event_id) {
            let _ = state.conn.send(&[0x80 | chan, note, 0]);
            state.note_leases.remove(&(chan, note));
        }
    }
}

fn get_delay(time: OscTime) -> Duration {
    let ntp_secs = time.seconds;
    if ntp_secs < 2_208_988_800 { return Duration::ZERO; }
    
    let unix_secs = (ntp_secs - 2_208_988_800) as u64;
    let nanos = ((time.fractional as f64 / 4_294_967_296.0) * 1_000_000_000.0) as u32;
    let event_time = UNIX_EPOCH + Duration::new(unix_secs, nanos);

    event_time.duration_since(SystemTime::now()).unwrap_or(Duration::ZERO)
}

fn osc_to_f32(val: &OscType) -> Option<f32> {
    match val {
        OscType::Float(f) => Some(*f),
        OscType::Double(d) => Some(*d as f32),
        OscType::Int(i) => Some(*i as f32),
        OscType::Long(l) => Some(*l as f32),
        _ => None,
    }
}
