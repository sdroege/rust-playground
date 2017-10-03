extern crate playground;
extern crate ipc_channel;

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::io::BufReader;
use std::thread;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ipc_channel::ipc;

fn main() {
    let args: Vec<_> = env::args().collect();
    let filename: &str = if args.len() == 2 {
        args[1].as_ref()
    } else {
        panic!("Usage: test-player file_path")
    };

    playground::initialize();
    let p = playground::player::Player::new();

    let (sender, receiver) = ipc::channel().unwrap();
    p.register_event_handler(sender);

    let path = Path::new(filename);
    let display = path.display();

    let file = match File::open(&path) {
        Err(why) => panic!("couldn't open {}: {}", display, why.description()),
        Ok(file) => file,
    };

    if let Ok(metadata) = file.metadata() {
        p.set_input_size(metadata.len());
    }

    if !p.start() {
        panic!("couldn't start");
    }

    let p_clone = p.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    let t = thread::spawn(move || {
        let p = &p_clone;
        let mut buf_reader = BufReader::new(file);
        let mut buffer = [0; 8192];
        while !shutdown_clone.load(Ordering::Relaxed) {
            match buf_reader.read(&mut buffer[..]) {
                Ok(0) => {
                    println!("finished pushing data");
                    break;
                }
                Ok(size) => {
                    if !p.push_data(Vec::from(&buffer[0..size])) {
                        break;
                    }
                },
                Err(e) => {
                    eprintln!("Error: {}", e);
                    break;
                }
            }
        }
    });

    p.play();

    while let Ok(event) = receiver.recv() {
        match event {
            playground::player::PlayerEvent::EndOfStream => {
                println!("EOF");
                break;
            }
            playground::player::PlayerEvent::Error => {
                println!("Error");
                break;
            }
            playground::player::PlayerEvent::MetadataUpdated(ref m) => {
                println!("Metadata updated! {:?}", m);
            }
            playground::player::PlayerEvent::StateChanged(ref s) => {
                println!("State changed to {:?}", s);
            }
        }
    }

    shutdown.store(true, Ordering::Relaxed);
    let _ = t.join();

    p.stop();
}
