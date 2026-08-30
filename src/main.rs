use macroquad::prelude::*;
use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::mpsc;
use std::time::Instant;

// --- CONSTANTS ---
const DRUM_CHANNEL: u8 = 9; // Note: MIDI channels are 0-indexed internally, so this is Channel 10
const NUM_CHANNELS: usize = 16;
const NUM_PITCHES: usize = 128;
const NUM_CONTROLLERS: usize = 128;

const NOTE_SPEED_PX_PER_SEC: f32 = 300.0;
const CC_HUD_TIMEOUT_SEC: f64 = 3.0;

const KEY_HEIGHT: f32 = 80.0;
const NUM_WHITE_KEYS: f32 = 52.0;

// --- TYPES ---
#[derive(Clone, Copy, Debug)]
enum MidiMessage {
    NoteOn {
        channel: u8,
        pitch: u8,
        velocity: u8,
        timestamp: f64,
    },
    NoteOff {
        channel: u8,
        pitch: u8,
        timestamp: f64,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
        timestamp: f64,
    },
}

struct NoteInfo {
    channel: u8,
    pitch: u8,
    velocity: u8,
    start_time: f64,
    end_time: Option<f64>,
}

// --- APP STATE & LOGIC ---
struct PianoRollApp {
    notes: Vec<NoteInfo>,
    active_pitches: [[u8; NUM_PITCHES]; NUM_CHANNELS],
    cc_values: [[Option<(u8, f64)>; NUM_CONTROLLERS]; NUM_CHANNELS],
    
    show_drums: bool,
    show_cc: bool,
    show_hints: bool,
    show_legend: bool,
    show_velocity: bool,
}

impl PianoRollApp {
    fn new() -> Self {
        Self {
            notes: Vec::new(),
            active_pitches: [[0u8; NUM_PITCHES]; NUM_CHANNELS],
            cc_values: [[None; NUM_CONTROLLERS]; NUM_CHANNELS],
            show_drums: true,
            show_cc: true,
            show_hints: false,
            show_legend: false,
            show_velocity: false,
        }
    }

    fn update(&mut self, rx: &mpsc::Receiver<MidiMessage>, current_time: f64) {
        // Toggle states
        if is_key_pressed(KeyCode::D) { self.show_drums = !self.show_drums; }
        if is_key_pressed(KeyCode::C) { self.show_cc = !self.show_cc; }
        if is_key_pressed(KeyCode::L) { self.show_legend = !self.show_legend; }
        if is_key_pressed(KeyCode::V) { self.show_velocity = !self.show_velocity; }
        if is_key_pressed(KeyCode::Slash) { self.show_hints = !self.show_hints; }

        // Process MIDI messages
        while let Ok(msg) = rx.try_recv() {
            match msg {
                MidiMessage::NoteOn { channel, pitch, velocity, timestamp } => {
                    self.notes.push(NoteInfo {
                        channel,
                        pitch,
                        velocity,
                        start_time: timestamp,
                        end_time: None,
                    });
                    self.active_pitches[channel as usize][pitch as usize] = velocity;
                }
                MidiMessage::NoteOff { channel, pitch, timestamp } => {
                    self.active_pitches[channel as usize][pitch as usize] = 0;
                    if let Some(note) = self.notes
                        .iter_mut()
                        .rev()
                        .find(|n| n.pitch == pitch && n.channel == channel && n.end_time.is_none())
                    {
                        note.end_time = Some(timestamp);
                    }
                }
                MidiMessage::ControlChange { channel, controller, value, timestamp } => {
                    self.cc_values[channel as usize][controller as usize] = Some((value, timestamp));
                }
            }
        }

        // Clean up expired CC values
        for ch in 0..NUM_CHANNELS {
            for v in self.cc_values[ch].iter_mut() {
                if let Some((_, ts)) = v {
                    if current_time - *ts > CC_HUD_TIMEOUT_SEC {
                        *v = None;
                    }
                }
            }
        }

        // Remove notes that have completely scrolled off screen
        let screen_h = screen_height();
        self.notes.retain(|n| {
            if let Some(et) = n.end_time {
                ((current_time - et) * NOTE_SPEED_PX_PER_SEC as f64) < screen_h as f64
            } else {
                true
            }
        });
    }

    fn draw(&self, current_time: f64) {
        clear_background(Color::new(0.1, 0.1, 0.12, 1.0));

        let screen_w = screen_width();
        let screen_h = screen_height();

        let drum_highway_w = if self.show_drums { 300.0_f32.min(screen_w * 0.3) } else { 0.0 };
        let piano_w = screen_w - drum_highway_w;

        let drum_x_start = 0.0;
        let piano_x_start = drum_highway_w;

        let white_key_width = piano_w / NUM_WHITE_KEYS;
        let black_key_width = white_key_width * 0.6;

        self.draw_falling_notes(current_time, screen_h, drum_highway_w, drum_x_start, piano_x_start, white_key_width, black_key_width);
        
        self.draw_piano_keys(screen_h, piano_x_start, white_key_width, black_key_width);
        
        if self.show_drums {
            self.draw_drum_pads(screen_h, drum_highway_w, drum_x_start);
        }
        
        self.draw_hud(screen_w);
    }

    fn draw_falling_notes(&self, current_time: f64, screen_h: f32, drum_highway_w: f32, drum_x_start: f32, piano_x_start: f32, white_key_width: f32, black_key_width: f32) {
        for note in &self.notes {
            if note.channel == DRUM_CHANNEL {
                if self.show_drums {
                    if let Some((_, lane)) = get_drum_lane(note.pitch) {
                        let lane_w = drum_highway_w / 8.0;
                        let center_x = drum_x_start + (lane as f32 * lane_w) + (lane_w / 2.0);
                        let y = screen_h - KEY_HEIGHT - ((current_time - note.start_time) * NOTE_SPEED_PX_PER_SEC as f64) as f32;

                        if y > screen_h || y < -50.0 { continue; }

                        let color = get_channel_color(note.channel, note.velocity, 1.0);
                        draw_circle(center_x, y, lane_w * 0.3, color);

                        if self.show_velocity {
                            self.draw_velocity_text(note.velocity, center_x, y + 2.0);
                        }
                    }
                }
            } else {
                let (is_black, white_idx) = get_key_pos(note.pitch);
                let center_x = piano_x_start + white_idx * white_key_width + (white_key_width / 2.0);
                
                let note_width = if is_black { black_key_width } else { white_key_width - 2.0 };
                let x = center_x - (note_width / 2.0);

                let end_t = note.end_time.unwrap_or(current_time);
                let y_bottom = screen_h - KEY_HEIGHT - ((current_time - end_t) * NOTE_SPEED_PX_PER_SEC as f64) as f32;
                let y_top = screen_h - KEY_HEIGHT - ((current_time - note.start_time) * NOTE_SPEED_PX_PER_SEC as f64) as f32;

                let y = y_top;
                let height = (y_bottom - y_top).max(3.0);

                if y > screen_h || y + height < 0.0 { continue; }

                let color = get_channel_color(note.channel, note.velocity, 1.0);
                
                // Note body
                draw_rectangle(x, y, note_width, height, color);
                
                // Note borders
                let border_color = Color::new(color.r * 0.6, color.g * 0.6, color.b * 0.6, color.a);
                draw_rectangle_lines(x, y, note_width, height, 1.0, border_color);

                // Note strike cap
                let cap_color = Color::new((color.r * 1.5).min(1.0), (color.g * 1.5).min(1.0), (color.b * 1.5).min(1.0), color.a);
                draw_rectangle(x, y + height - 2.0, note_width, 2.0, cap_color);

                if self.show_velocity { 
                    self.draw_velocity_text(note.velocity, center_x, y + height - 3.0);
                }
            }
        }
    }

    fn draw_piano_keys(&self, screen_h: f32, piano_x_start: f32, white_key_width: f32, black_key_width: f32) {
        // Draw white keys
        for i in 21..=108 {
            let (is_black, white_idx) = get_key_pos(i);
            if !is_black {
                let color = self.get_active_key_color(i).unwrap_or(BLACK);
                let x = piano_x_start + white_idx * white_key_width;
                
                draw_rectangle(x, screen_h - KEY_HEIGHT, white_key_width, KEY_HEIGHT, color);
                draw_rectangle_lines(x, screen_h - KEY_HEIGHT, white_key_width, KEY_HEIGHT, 1.0, Color::new(0.2, 0.2, 0.2, 1.0));
            }
        }

        // Draw black keys
        for i in 21..=108 {
            let (is_black, white_idx) = get_key_pos(i);
            if is_black {
                let color = self.get_active_key_color(i).unwrap_or(Color::new(0.1, 0.1, 0.1, 1.0));
                let center_x = piano_x_start + white_idx * white_key_width + (white_key_width / 2.0);
                let x = center_x - (black_key_width / 2.0);
                
                draw_rectangle(x, screen_h - KEY_HEIGHT + 1.0, black_key_width, KEY_HEIGHT * 0.65, color);
            }
        }
    }

    fn draw_drum_pads(&self, screen_h: f32, drum_highway_w: f32, drum_x_start: f32) {
        let lane_w = drum_highway_w / 8.0;
        for lane in 0..8 {
            let x = drum_x_start + (lane as f32 * lane_w);

            let max_vel = self.active_pitches[DRUM_CHANNEL as usize]
                .iter()
                .enumerate()
                .filter_map(|(p, &vel)| {
                    if vel > 0 && get_drum_lane(p as u8).map(|(_, l)| l) == Some(lane) { Some(vel) } else { None }
                })
                .max();

            let color = if let Some(vel) = max_vel {
                get_channel_color(DRUM_CHANNEL, vel, 1.0)
            } else {
                Color::new(0.2, 0.2, 0.2, 1.0)
            };

            draw_rectangle(x, screen_h - KEY_HEIGHT, lane_w - 2.0, KEY_HEIGHT, color);
            draw_rectangle_lines(x, screen_h - KEY_HEIGHT, lane_w - 2.0, KEY_HEIGHT, 1.0, GRAY);

            let label = match lane {
                0 => "KICK", 1 => "SNR", 2 => "CHH", 3 => "OHH", 
                4 => "TOM", 5 => "CRSH", 6 => "RIDE", _ => "PERC",
            };

            let text_size = measure_text(label, None, 16u16, 1.0);
            let text_x = x + (lane_w / 2.0) - (text_size.width / 2.0);
            draw_text(label, text_x, screen_h - (KEY_HEIGHT / 2.0) + (text_size.height / 2.0), 16.0, WHITE);
        }
    }

    fn draw_hud(&self, screen_w: f32) {
        let mut cc_text_y = if self.show_hints { 150.0 } else { 30.0 };
        let cc_text_x = screen_w - 280.0;

        let ccs_active = self.cc_values.iter().any(|ch_array| ch_array.iter().any(|v| v.is_some()));

        if self.show_cc && ccs_active {
            draw_text("MIDI CC Monitor", cc_text_x, cc_text_y, 20.0, WHITE);
            cc_text_y += 25.0;

            for ch in 0..NUM_CHANNELS {
                for (controller, value_opt) in self.cc_values[ch].iter().enumerate() {
                    if let Some((value, _)) = *value_opt {
                        let text = format!("CH {:02} | CC {:03}: {:03}", ch + 1, controller, value);
                        draw_text(&text, cc_text_x, cc_text_y, 16.0, LIGHTGRAY);

                        let bar_width = 100.0;
                        let fill_width = (value as f32 / 127.0) * bar_width;
                        let bar_x = cc_text_x + 145.0;

                        draw_rectangle(bar_x, cc_text_y - 12.0, bar_width, 10.0, Color::new(0.2, 0.2, 0.2, 0.8));
                        draw_rectangle(bar_x, cc_text_y - 12.0, fill_width, 10.0, Color::new(0.0, 0.8, 0.5, 0.8));

                        cc_text_y += 20.0;
                    }
                }
            }
        }

        if self.show_hints {
            let hints = [
                "[?] Toggle Hints".to_string(),
                format!("[D] Drums: {}", if self.show_drums { "ON" } else { "OFF" }),
                format!("[C] CC Monitor: {}", if self.show_cc { "ON" } else { "OFF" }),
                format!("[L] Legend: {}", if self.show_legend { "ON" } else { "OFF" }),
                format!("[V] Velocity: {}", if self.show_velocity { "ON" } else { "OFF" })
            ];

            let max_w = hints.iter().map(|h| measure_text(h, None, 20, 1.0).width).fold(0.0, f32::max);
            let hints_x = screen_w - max_w - 15.0;
            let mut hints_y = 30.0;

            for hint in hints {
                draw_text(&hint, hints_x, hints_y, 20.0, WHITE);
                hints_y += 25.0;
            }
        }

        if self.show_legend {
            let legend_x = 20.0;
            let mut legend_y = 30.0;

            draw_rectangle(legend_x - 10.0, legend_y - 20.0, 150.0, 390.0, Color::new(0.0, 0.0, 0.0, 0.6));
            draw_text("Channels", legend_x, legend_y, 20.0, WHITE);
            legend_y += 15.0;

            for ch in 0..NUM_CHANNELS as u8 {
                let color = get_channel_color(ch, 127, 1.0);
                draw_rectangle(legend_x, legend_y, 16.0, 16.0, color);
                draw_rectangle_lines(legend_x, legend_y, 16.0, 16.0, 1.0, GRAY);
                
                let label = if ch == DRUM_CHANNEL { format!("CH 10 (Drums)") } else { format!("CH {}", ch + 1) };
                draw_text(&label, legend_x + 25.0, legend_y + 13.0, 16.0, WHITE);
                legend_y += 22.0;
            }
        }
    }

    // Small helper to reuse logic for drawing a drop-shadowed velocity value
    fn draw_velocity_text(&self, velocity: u8, anchor_x: f32, anchor_y: f32) {
        let vel_text = format!("{}", velocity);
        let text_size = measure_text(&vel_text, None, 14, 1.0);
        let text_x = anchor_x - (text_size.width / 2.0);
        
        draw_text(&vel_text, text_x + 1.0, anchor_y + 1.0, 14.0, Color::new(0.0, 0.0, 0.0, 0.8));
        draw_text(&vel_text, text_x, anchor_y, 14.0, WHITE);
    }

    // Finds if any non-drum channel is currently pressing this pitch
    fn get_active_key_color(&self, pitch: u8) -> Option<Color> {
        for ch in 0..NUM_CHANNELS {
            if ch as u8 == DRUM_CHANNEL { continue; }
            let vel = self.active_pitches[ch][pitch as usize];
            if vel > 0 {
                return Some(get_channel_color(ch as u8, vel, 1.0));
            }
        }
        None
    }
}

// --- HELPER FUNCTIONS ---
fn setup_midi(
    tx: mpsc::Sender<MidiMessage>,
    app_start: Instant,
) -> Result<MidiInputConnection<()>, Box<dyn std::error::Error>> {
    let mut midi_in = MidiInput::new("Piano Roll Input")?;
    midi_in.ignore(Ignore::None);

    let in_ports = midi_in.ports();
    if in_ports.is_empty() { return Err("No MIDI input ports found.".into()); }

    println!("Available MIDI ports:");
    for (i, p) in in_ports.iter().enumerate() {
        println!("{}: {}", i, midi_in.port_name(p)?);
    }

    let in_port = in_ports
        .iter()
        .find(|p| midi_in.port_name(p).unwrap_or_default().contains("Midi Through"))
        .unwrap_or_else(|| in_ports.first().unwrap());

    println!("Connecting to: {}", midi_in.port_name(in_port)?);

    let conn = midi_in.connect(
        in_port,
        "midir-read-input",
        move |_, message, _| {
            let timestamp = app_start.elapsed().as_secs_f64();
            if message.len() >= 3 {
                let status = message[0] & 0xF0;
                let channel = message[0] & 0x0F;
                let data1 = message[1];
                let data2 = message[2];

                if status == 0x90 {
                    if data2 > 0 {
                        let _ = tx.send(MidiMessage::NoteOn { channel, pitch: data1, velocity: data2, timestamp });
                    } else {
                        let _ = tx.send(MidiMessage::NoteOff { channel, pitch: data1, timestamp });
                    }
                } else if status == 0x80 {
                    let _ = tx.send(MidiMessage::NoteOff { channel, pitch: data1, timestamp });
                } else if status == 0xB0 {
                    let _ = tx.send(MidiMessage::ControlChange { channel, controller: data1, value: data2, timestamp });
                }
            }
        },
        (),
    )?;

    Ok(conn)
}

fn get_key_pos(pitch: u8) -> (bool, f32) {
    if pitch < 21 || pitch > 108 { return (false, 0.0); }

    let notes_in_octave = [
        (false, 0.0), (true, 0.5), (false, 1.0), (true, 1.5),
        (false, 2.0), (false, 3.0), (true, 3.5), (false, 4.0),
        (true, 4.5), (false, 5.0), (true, 5.5), (false, 6.0),
    ];

    let note_in_octave = pitch % 12;
    let octave = (pitch / 12) as f32;
    let (is_black, rel_pos) = notes_in_octave[note_in_octave as usize];

    let absolute_white_idx = octave * 7.0 + rel_pos;
    let adjusted_idx = absolute_white_idx - 12.0;

    (is_black, adjusted_idx)
}

fn get_channel_color(channel: u8, velocity: u8, alpha: f32) -> Color {
    let colors = [
        (1.0, 0.3, 0.3), (0.3, 1.0, 0.3), (0.3, 0.5, 1.0), (1.0, 1.0, 0.3), 
        (1.0, 0.6, 0.2), (0.8, 0.3, 0.8), (0.3, 1.0, 1.0), (1.0, 0.4, 0.7), 
        (0.6, 0.8, 0.2), (0.4, 0.8, 1.0), (0.9, 0.2, 0.5), (0.2, 0.8, 0.6), 
        (0.7, 0.4, 0.0), (0.6, 0.6, 0.6), (0.8, 0.8, 0.9), (1.0, 0.8, 0.6), 
    ];
    let (r, g, b) = colors[(channel as usize) % 16];
    let intensity = (velocity as f32 / 127.0).clamp(0.2, 1.0);
    Color::new(r * intensity, g * intensity, b * intensity, alpha)
}

fn get_drum_lane(pitch: u8) -> Option<(&'static str, usize)> {
    match pitch {
        35 | 36 => Some(("KICK", 0)),
        38 | 40 => Some(("SNR", 1)),
        42 | 44 => Some(("CHH", 2)), 
        46 => Some(("OHH", 3)),      
        41 | 43 | 45 | 47 | 48 | 50 => Some(("TOM", 4)),
        49 | 52 | 55 | 57 => Some(("CRSH", 5)),
        51 | 53 | 59 => Some(("RIDE", 6)),
        _ => Some(("PERC", 7)), 
    }
}

// --- MAIN LOOP ---
#[macroquad::main("MIDI Piano Roll")]
async fn main() {
    let app_start = Instant::now();
    let (tx, rx) = mpsc::channel();

    let _midi_conn = match setup_midi(tx, app_start) {
        Ok(conn) => Some(conn),
        Err(e) => {
            eprintln!("Failed to setup MIDI: {}", e);
            None
        }
    };

    let mut app = PianoRollApp::new();

    loop {
        let current_time = app_start.elapsed().as_secs_f64();
        
        app.update(&rx, current_time);
        app.draw(current_time);

        next_frame().await
    }
}
