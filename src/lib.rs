extern crate gstreamer as gst;

pub mod player;

pub fn initialize() {
    // TODO: error handling.
    gst::init().unwrap();
}
