# Twilio WebSocket Audio Processor
This Rust application demonstrates handling, processing, and playing back Twilio phone call audio data received over a live duplex WebSocket connection. Utilizing the `warp` web server framework for asynchronous WebSocket communication, the application efficiently processes batches of audio data encoded in base64 format. Key functionalities include audio data queue management, batch processing for efficiency, raw audio file creation, format conversion (from uLaw to WAV), and playback of base64-encoded audio chunks.

## Features

- **WebSocket Communication**: Leverages `warp` for efficient WebSocket handling.
- **Audio Queue Management**: Utilizes a global, thread-safe queue to manage incoming audio data.
- **Batch Processing**: Processes audio data in batches to optimize performance.
- **Audio Conversion**: Converts raw uLaw audio files to WAV format using `ffmpeg`.
- **Audio Playback**: Plays back base64-encoded audio chunks using `rodio`.

## Installation

Ensure you have Rust and Cargo installed on your system. Additionally, `ffmpeg` must be installed and accessible in your system's PATH for audio conversion.


## Dependencies

- [warp](https://crates.io/crates/warp): For WebSocket and web server functionality.
- [tokio](https://crates.io/crates/tokio): Asynchronous runtime.
- [rodio](https://crates.io/crates/rodio): For audio playback.
- [serde_json](https://crates.io/crates/serde_json): For JSON parsing.
- [lazy_static](https://crates.io/crates/lazy_static): For defining lazy-initialized static variables.
- [log](https://crates.io/crates/log): For logging.