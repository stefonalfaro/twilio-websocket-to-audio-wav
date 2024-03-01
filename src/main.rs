extern crate lazy_static;
use futures::StreamExt;
use log::{info, warn};
use warp::ws::{Message, WebSocket};
use warp::Filter;
use std::env;
use base64;
use std::process::Command;
use std::io::{self};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use std::fs;
use std::sync::Arc;
use std::collections::HashMap;

//Confiuration for Level 1 algorithm that analyzes the audio stream to detect speech when the amplitude is consectively above the AMPLITUDE_THRESHOLD for SEQUENCE_THRESHOLD
const SPEACH_DETECTION_AMPLITUDE_THRESHOLD: i16 = 5000;
const SPEACH_DETECTION_SEQUENCE_THRESHOLD: usize = 5;

//Configuration for Level 1 algorithm that analyzes the audio stream to detect pauses when the amplitude is consectively below the AMPLITUDE_THRESHOLD for SEQUENCE_THRESHOLD
const PAUSE_DETECTION_AMPLITUDE_THRESHOLD: i16 = 2000;
const PAUSE_DETECTION_SEQUENCE_THRESHOLD: usize = 15;

//Level 2 algorithm based upon both Level 1 algorithms to detect the start and end of an audio clip based on the amount of times the spech or pauses are consequtively triggered.
const MIN_CONSECUTIVE_SPEECH_COUNT: usize = 5;
const MIN_CONSECUTIVE_PAUSE_COUNT: usize = 3;
const OFFSET:i32 = 10;

lazy_static! {
    static ref AUDIO_QUEUE: Mutex<Vec<(i32, String)>> = Mutex::new(Vec::new());

    static ref PAUSE_QUEUE: Mutex<Vec<i16>> = Mutex::new(Vec::new());
    
    static ref SPEACH_QUEUE: Mutex<Vec<i16>> = Mutex::new(Vec::new());

    static ref AUDIO_CLIP_STATE: Mutex<AudioState> = Mutex::new(AudioState::Idle);
    static ref CONSECUTIVE_SPEECH_COUNT: Mutex<usize> = Mutex::new(0);
    static ref CONSECUTIVE_PAUSE_COUNT: Mutex<usize> = Mutex::new(0);
    static ref CURRENT_CLIP_START: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));

    static ref MESSAGES: Mutex<HashMap<i32, String>> = Mutex::new(HashMap::new());
}

#[derive(Debug, PartialEq)]
enum AudioState {
    Idle,
    InSpeech,
    PostSpeech,
}

//Start a webserver to handle websockets
#[tokio::main]
async fn main() {
    env::set_var("RUST_LOG", "info"); //info or warn
    env_logger::init();

    // Define the WebSocket route
    let media = warp::path("media")
        .and(warp::ws())
        .map(|ws: warp::ws::Ws| {
            ws.on_upgrade(|socket| handle_connection(socket))
        });

    // Start the server
    let port = 5000;
    warp::serve(media)
        .run(([0, 0, 0, 0], port))
        .await;
}

//1 Handle WebSocket connection and pass messages to the handle_message
async fn handle_connection(ws: WebSocket) {
    warn!("Connection accepted");

    let (_sender, mut receiver) = ws.split();

    let mut message_count: u64 = 0;

    while let Some(result) = receiver.next().await {
        match result {
            Ok(msg) => {
                handle_message(&msg).await;
                message_count += 1;
            }
            Err(e) => {
                warn!("WebSocket error: {}", e);
                break;
            }
        }
    }

    warn!("Connection closed. Received a total of {} messages", message_count);
}

//2 This is called everytime the websocket sends something in the stream. Hundreds in a few seconds. The sequenceNumber is how we can queue them up.
async fn handle_message(msg: &Message) {
    // Assuming `handle_message` can access the queue
    if msg.is_text() {
        let message = msg.to_str().unwrap();
        let data: serde_json::Value = serde_json::from_str(message).unwrap();

        if let Some("media") = data["event"].as_str() {
            let sequence_number = data["sequenceNumber"].as_str().unwrap().parse::<i32>().unwrap();
            let payload = data["media"]["payload"].as_str().unwrap();  
            let audio_chunk = base64::decode(payload).expect("Failed to decode base64");

            //1 Run the calculate amplitude and add to message queue in parallel
            let (amplitude, _) = tokio::join!(
                calculate_amplitude(&audio_chunk),
                insert_message_async(sequence_number, payload.to_string())
            );

            //2 Run the Pause and Speach detection algorithms in parallel
            let (pause_detected, speach_detected) = tokio::join!(
                pause_detection(amplitude),
                speach_detection(amplitude)
            );

            //2.1 Start and end detection. This is how we would know to send the .wav file to our voice to text service
            if speach_detected || pause_detected //Import we only do this when there is a Speach detected or Pause detected event
            {
                let mut state = AUDIO_CLIP_STATE.lock().await;
                let mut speech_count = CONSECUTIVE_SPEECH_COUNT.lock().await;
                let mut pause_count = CONSECUTIVE_PAUSE_COUNT.lock().await;
                let mut clip_start = CURRENT_CLIP_START.lock().await;
                match *state {
                    AudioState::Idle => {
                        info!("State: Idle, Speech Count: {}", *speech_count);
                        if speach_detected {
                            *speech_count += 1;
                            if *speech_count >= MIN_CONSECUTIVE_SPEECH_COUNT {
                                *state = AudioState::InSpeech;
                                *clip_start = Some(sequence_number); // Mark the start of speech
                                warn!("Clip started at sequence: {}", sequence_number - OFFSET);
                                *speech_count = 0; // Reset speech count after transitioning
                            }
                        } else {
                            // If no speech is detected, keep the state as Idle and reset speech count
                            *speech_count = 0;
                        }
                    },
                    AudioState::InSpeech => {
                        info!("State: InSpeech, Speech Count: {}, Pause Count: {}", *speech_count, *pause_count);
                        if pause_detected {
                            *pause_count += 1;
                            if *pause_count >= MIN_CONSECUTIVE_PAUSE_COUNT {
                                *state = AudioState::PostSpeech;
                                warn!("Clip ended at sequence: {}", sequence_number + OFFSET);

                                // Here you can process the audio clip defined by clip_start and this sequence_number
                                *pause_count = 0; // Reset pause count after confirming the end of a clip
                            }
                        } else {
                            // If speech continues, reset pause count to wait for a clear end of the clip
                            *pause_count = 0;
                        }
                    },
                    AudioState::PostSpeech => {
                        if let Some(start) = *clip_start {
                            let end = sequence_number; 
                            warn!("Processing from sequence {} to {}", start, end);
                            
                            // Spawn a new asynchronous task to process the audio clip
                            let start_clone = start; // Clone data as needed for the async block
                            let end_clone = end;
                            tokio::spawn(async move {
                                process_audio_clip(start_clone, end_clone).await;
                            });
                        }
                    
                        info!("State: PostSpeech, Resetting for next clip");
                        // After processing the clip, prepare for the next potential clip
                        *state = AudioState::Idle;
                        *clip_start = None; // Clear the start marker for the next clip
                        // No need to reset speech_count or pause_count here as they are reset upon transitions to InSpeech or upon confirming a clip end
                    },
                }
            }
        }
    }
}

//Add a message to the hashmap against the sequence
async fn insert_message_async(sequence_number: i32, payload: String) {
    let mut messages = MESSAGES.lock().await;
    messages.insert(sequence_number, payload);
}

//This is called once we have a batch of messages in a row. We write the raw ulaw file and then convert it to a wave using the next function.
async fn process_audio_batch(batch: Vec<(i32, String)>, last_sequence: i32) {
    // Concatenate base64 audio data
    let audio_data = batch.iter().map(|(_seq, data)| base64::decode(data).unwrap()).flatten().collect::<Vec<_>>();

    // Generate file names based on the last sequence number of the batch
    let raw_filename = format!("audio/{}.raw", last_sequence);
    let wav_filename = format!("audio/{}.wav", last_sequence);

    // Save raw audio data to a file
    std::fs::write(&raw_filename, audio_data).expect("Failed to write raw audio data");

    // Convert the raw audio file to WAV format
    convert_raw_to_wav(&raw_filename, &wav_filename).expect("Failed to convert raw to WAV");
}

//This converts the ulaw raw to a wav file
fn convert_raw_to_wav(raw_path: &str, wav_path: &str) -> io::Result<()> {
    let output = Command::new("ffmpeg")
        .args([
            "-f", "mulaw",
            "-y", // Automatically overwrite existing files without asking
            "-ar", "16000",
            "-ac", "1",
            "-i", raw_path,
            wav_path,
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("ffmpeg error: {}", String::from_utf8_lossy(&output.stderr));
        return Err(io::Error::new(io::ErrorKind::Other, "ffmpeg conversion failed"));
    }

    warn!("Audio file '{}' created.", wav_path);

    // Delete the old raw file
    match fs::remove_file(raw_path) {
        Ok(_) => {
            info!("Successfully deleted {}", raw_path)
        },
        Err(e) => eprintln!("Failed to delete {}: {}", raw_path, e),
    }

    Ok(())
}

//Convert ulaw encoded bytes to linear pulse code modulation values
fn mulaw_to_pcm(mulaw: u8) -> i16 {
    let mulaw = !mulaw;
    let exponent = (mulaw >> 4) & 0x07;
    // Ensure `sample` starts as an i16 to support negation later.
    let mut sample = ((mulaw & 0x0F) as i16) << 1 | 0x21;
    sample <<= exponent + 2;
    sample -= 0x21;

    if mulaw & 0x80 != 0 {
        sample = -sample;
    }

    sample
}

//Finding the maximum absolute value of the PCM samples in a chunk, which gives you the peak amplitude.
async fn calculate_amplitude(chunk: &[u8]) -> i16 {
    chunk.iter()
        .map(|&mulaw| mulaw_to_pcm(mulaw))
        .map(|pcm| pcm.abs())
        .max()
        .unwrap_or(0)
}

async fn process_audio_clip(start_sequence: i32, end_sequence: i32) {
    let messages = MESSAGES.lock().await; // Assuming MESSAGES is a Mutex<HashMap<i32, String>>
    let mut batch: Vec<(i32, String)> = Vec::new();

    // Collect messages within the specified range
    for sequence_number in start_sequence..=end_sequence {
        if let Some(message) = messages.get(&sequence_number) {
            batch.push((sequence_number, message.clone())); // Clone the message string
        }
    }

    // Now that you have the batch, you can process it
    if !batch.is_empty() {
        info!("Collected the sequence.");
        process_audio_batch(batch, end_sequence).await; // Adjust this call based on your actual function signature
    }
}

//Pause detection algorithm.
async fn pause_detection(amplitude: i16) -> bool {
    let mut pause_queue = PAUSE_QUEUE.lock().await;
    pause_queue.push(amplitude);
    if pause_queue.len() > PAUSE_DETECTION_SEQUENCE_THRESHOLD {
        pause_queue.remove(0);
    }
    let pause_detected = pause_queue.iter()
        .filter(|&&amp| amp < PAUSE_DETECTION_AMPLITUDE_THRESHOLD)
        .count() >= PAUSE_DETECTION_SEQUENCE_THRESHOLD;
    if pause_detected {
        info!("Pause detected.");
        pause_queue.clear();
    }
    pause_detected
}

//Speach detection algorithm
async fn speach_detection(amplitude: i16) -> bool {
    let mut speech_queue = SPEACH_QUEUE.lock().await;
    speech_queue.push(amplitude);
    if speech_queue.len() > SPEACH_DETECTION_SEQUENCE_THRESHOLD {
        speech_queue.remove(0);
    }
    let speech_detected = speech_queue.iter()
        .filter(|&&amp| amp > SPEACH_DETECTION_AMPLITUDE_THRESHOLD)
        .count() >= SPEACH_DETECTION_SEQUENCE_THRESHOLD;
    if speech_detected {
        info!("Speech detected.");
        speech_queue.clear();
    }
    speech_detected
}
