extern crate gstreamer as gst;

#[macro_use]
extern crate serde_derive;

pub mod player;

pub fn initialize() {
    // TODO: error handling.
    gst::init().unwrap();
}
