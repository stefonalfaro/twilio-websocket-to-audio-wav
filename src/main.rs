#[macro_use]
extern crate lazy_static;
use futures::StreamExt;
use log::{info, warn};
use warp::ws::{Message, WebSocket};
use warp::Filter;
use std::env;
use std::io::Cursor;
use rodio::{Decoder, OutputStream, Source}; // Add `Source` here
use base64;
use std::process::Command;
use std::io::{self};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use std::fs;

//Start a webserver to handle websockets
#[tokio::main]
async fn main() {
    env::set_var("RUST_LOG", "info");
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

lazy_static! {
    static ref AUDIO_QUEUE: Mutex<Vec<(i32, String)>> = Mutex::new(Vec::new());
}

//1 Handle WebSocket connection and pass messages to the handle_message
async fn handle_connection(ws: WebSocket) {
    info!("Connection accepted");

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

    info!("Connection closed. Received a total of {} messages", message_count);
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

            // Queue the message
            let mut queue = AUDIO_QUEUE.lock().await;
            queue.push((sequence_number, payload.to_string()));

            // Process in batches
            if queue.len() >= 250 {
                // Example: Get the last sequence number in the batch for naming
                let last_sequence = queue.last().unwrap().0;
                let batch = queue.drain(..).collect::<Vec<_>>();
                process_audio_batch(batch, last_sequence).await;
            }
        }
    }
}

//3 This is called once we have a batch of messages in a row. We write the raw ulaw file and then convert it to a wave using the next function.
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

//4 This converts the ulaw raw to a wav file
fn convert_raw_to_wav(raw_path: &str, wav_path: &str) -> io::Result<()> {
    let output = Command::new("ffmpeg")
        .args([
            "-f", "mulaw",
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

    info!("{} created.", wav_path);

    // Delete the old raw file
    match fs::remove_file(raw_path) {
        Ok(_) => info!("Successfully deleted {}", raw_path),
        Err(e) => eprintln!("Failed to delete {}: {}", raw_path, e),
    }

    Ok(())
}

// Function to play a base64-encoded audio chunk
fn play_base64_audio_chunk(base64_audio: &str) {
    // Decode the base64 string
    let audio_bytes = base64::decode(base64_audio).expect("Failed to decode base64 audio");

    // You would typically concatenate these bytes with previously received bytes here

    // For demonstration, play the decoded audio chunk immediately
    // Note: This assumes the audio is in a format that the `Decoder` can understand
    let cursor = Cursor::new(audio_bytes);
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let source = Decoder::new(cursor).unwrap();
    stream_handle.play_raw(source.convert_samples()).unwrap();

    // Note: In a real application, you'd want to manage playback more gracefully
}