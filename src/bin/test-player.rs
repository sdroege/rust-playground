extern crate playground;

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::io::BufReader;
use std::{thread, time};
use std::sync::{Arc, Mutex};

fn main() {

    let args: Vec<_> = env::args().collect();
    let filename: &str = if args.len() == 2 {
        args[1].as_ref()
    } else {
        panic!("Usage: test-player file_path")
    };

    playground::initialize();
    let mut p = playground::player::Player::new();

    let end_of_stream = Arc::new(Mutex::new(false));
    let inner_eos = end_of_stream.clone();
    p.register_event_handler(move |event| match *event {
        playground::player::PlayerEvent::EndOfStream => {
            let inner = Arc::clone(&inner_eos);
            let mut eos_guard = inner.lock().unwrap();
            *eos_guard = true;
        }
        playground::player::PlayerEvent::MetadataUpdated(ref m) => {
            println!("Metadata updated! {:?}", m);
        }
    });

    let path = Path::new(filename);
    let display = path.display();

    let file = match File::open(&path) {
        Err(why) => panic!("couldn't open {}: {}", display, why.description()),
        Ok(file) => file,
    };

    if let Ok(metadata) = file.metadata() {
        p.set_input_size(metadata.len());
    }
    p.start();
    p.play();

    let mut buf_reader = BufReader::new(file);
    while !*end_of_stream.lock().unwrap() {
        let mut buffer = [0; 8192];
        match buf_reader.read(&mut buffer[..]) {
            Ok(size) => if size > 0 {
                if !p.push_data(&buffer) {
                    break;
                }
            } else {
                thread::sleep(time::Duration::from_millis(200));
            },
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }
    p.stop();
}
