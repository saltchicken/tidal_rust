use midir::{MidiOutput, MidiOutputConnection};
use rosc::{OscPacket, OscType};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup MIDI Output (Creates a Virtual ALSA Port on Linux)
    let midi_out = MidiOutput::new("Tidal Rust Client")?;
    let conn_out = midi_out.create_virtual("Tidal Midi Out")?;
    let midi_tx = Arc::new(Mutex::new(conn_out));

    // 2. Setup UDP Socket matching Tidal's default SuperDirt port
    let addr = "127.0.0.1:3819";
    let socket = UdpSocket::bind(addr).await?;
    println!("🚀 Rust Tidal server listening for OSC on {}", addr);
    println!("🎹 Created virtual ALSA MIDI port: 'Tidal Midi Out'");

    let mut buf = [0u8; 8192];

    loop {
        // Wait for incoming OSC packets
        let (len, _) = socket.recv_from(&mut buf).await?;
        
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..len]) {
            let midi_tx_clone = midi_tx.clone();
            
            // Spawn a task for each packet so we don't block the listener
            tokio::spawn(async move {
                handle_packet(packet, midi_tx_clone).await;
            });
        }
    }
}

// Recursively unpack OSC Bundles to get to the Messages
async fn handle_packet(packet: OscPacket, midi_tx: Arc<Mutex<MidiOutputConnection>>) {
    match packet {
        OscPacket::Message(msg) => {
            process_message(msg, midi_tx).await;
        }
        OscPacket::Bundle(bundle) => {
            for element in bundle.content {
                handle_packet(element, midi_tx.clone()).await;
            }
        }
    }
}

async fn process_message(msg: rosc::OscMessage, midi_tx: Arc<Mutex<MidiOutputConnection>>) {
    // Tidal sends arguments as a flat list of alternating keys and values
    // e.g., ["s", "rust", "midinote", 60.0, "midichan", 0.0]
    let mut map = HashMap::new();
    let mut i = 0;
    
    while i < msg.args.len().saturating_sub(1) {
        if let OscType::String(key) = &msg.args[i] {
            map.insert(key.clone(), msg.args[i + 1].clone());
        }
        i += 2;
    }

    // Only process events targeting the "rust" synth
    let s = map.get("s").and_then(|v| v.clone().string());
    if s.as_deref() != Some("rust") {
        return;
    }

    // Extract MIDI parameters with safe fallbacks
    let note = map.get("midinote").and_then(osc_to_f32).map(|n| n as u8).unwrap_or(60);
    let chan = map.get("midichan").and_then(osc_to_f32).map(|c| c as u8).unwrap_or(0);
    
    // Tidal's 'amp' is 0.0 to 1.0. Scale it to MIDI velocity 0-127.
    let amp = map.get("amp").and_then(osc_to_f32).unwrap_or(0.8);
    let velocity = (amp * 127.0).clamp(0.0, 127.0) as u8;
    
    // Duration: 'sustain' is usually sent by Tidal in seconds
    let duration_sec = map.get("sustain").and_then(osc_to_f32).unwrap_or(0.5);

    let chan = chan.clamp(0, 15);
    let note = note.clamp(0, 127);

    // 1. Send Note On
    {
        let mut conn = midi_tx.lock().await;
        let _ = conn.send(&[0x90 | chan, note, velocity]);
    }

    // 2. Wait for the note duration asynchronously
    sleep(Duration::from_secs_f32(duration_sec)).await;

    // 3. Send Note Off (Velocity 0)
    {
        let mut conn = midi_tx.lock().await;
        let _ = conn.send(&[0x80 | chan, note, 0]);
    }
}

// Helper to coerce different numerical OSC types into f32
fn osc_to_f32(val: &OscType) -> Option<f32> {
    match val {
        OscType::Float(f) => Some(*f),
        OscType::Double(d) => Some(*d as f32),
        OscType::Int(i) => Some(*i as f32),
        OscType::Long(l) => Some(*l as f32),
        _ => None,
    }
}
