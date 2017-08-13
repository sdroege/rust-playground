extern crate playground;

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::io::BufReader;
use std::{thread, time};

fn main() {

    let args: Vec<_> = env::args().collect();
    let filename: &str = if args.len() == 2 {
        args[1].as_ref()
    } else {
        panic!("Usage: test-player file_path");
    };

    playground::initialize();
    let mut p = playground::player::Player::new();
    p.start();

    let path = Path::new(filename);
    let display = path.display();

    let file = match File::open(&path) {
        Err(why) => panic!("couldn't open {}: {}", display, why.description()),
        Ok(file) => file,
    };

    if let Ok(metadata) = file.metadata() {
        p.set_input_size(metadata.len());
    }
    p.play();

    let mut buf_reader = BufReader::new(file);

    let mut metadata_found = false;
    while !p.end_of_stream() {
        let mut buffer = [0; 8192];
        if !p.ready() {
            thread::sleep(time::Duration::from_millis(200));
            continue;
        }
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

        if !metadata_found {
            if let Some(metadata) = p.get_metadata() {
                println!("Metadata: {:?}", metadata);
                metadata_found = true;
            }
        }
    }
    p.stop();
}
