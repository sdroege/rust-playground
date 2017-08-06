extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;
extern crate glib;
use self::glib::*;
use self::gst::{ElementExt, GstObjectExt, PadExt};

use std::u64;
use std::thread;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Metadata {
    width: i32,
    height: i32,
}

#[derive(Debug, Clone)]
struct PlayerInner {
    playbin: gst::Element,
    appsrc: Option<gst_app::AppSrc>,
    eos_reached: bool,
    metadata: Option<Metadata>,
}

#[derive(Debug)]
pub struct Player {
    inner: Arc<Mutex<PlayerInner>>,
}


impl PlayerInner {
    pub fn handle_eos(&mut self) {
        println!("EOS!");
        self.eos_reached = true;
    }

    pub fn stop(&mut self) {
        println!("Stop!");
        self.playbin.set_state(gst::State::Null);
    }

    pub fn get_metadata(&mut self) -> Option<Metadata> {
        // FIXME: hopefully a copy can be avoided here
        if let Some(m) = self.clone().metadata {
            Some(Metadata {
                width: m.width,
                height: m.height,
            })
        } else {
            None
        }
    }

    pub fn check_video_dimensions(&mut self, caps: gst::GstRc<gst::CapsRef>) {
        println!("caps {:?}", caps.to_string());
        // TODO: Switch this to GstVideoInfo
        if let Some(s) = caps.get_structure(0) {
            // FIXME: apply PAR
            let width = s.get::<i32>("width").unwrap();
            let height = s.get::<i32>("height").unwrap();
            self.metadata = Some(Metadata {
                width: width,
                height: height,
            });
        }

    }

    pub fn check_metadata(&mut self) {
        let video_sink = self.playbin
            .get_property("video-sink")
            .unwrap()
            .downcast::<gst::Element>()
            .unwrap()
            .get()
            .unwrap();
        if let Some(pad) = video_sink.get_static_pad("sink") {
            if let Some(caps) = pad.get_current_caps() {
                self.check_video_dimensions(caps);
            } else {
                // FIXME
                // pad.connect("notify::caps", true, move |caps| {
                //     self.check_video_dimensions(caps.get().unwrap());
                //     None
                // });
            }

        }
    }
}

impl Player {
    pub fn new() -> Player {
        // FIXME: glimagesink can't be used because:
        // 1. test-player isn't a Cocoa app running a NSApplication
        // 2. the GstGLDisplayCocoa depends on a main GLib loop in that case ^^ which test-player isn't using
        let playbin = gst::parse_launch(
            "playbin uri=appsrc:// video-sink=osxvideosink audio-sink=autoaudiosink",
        ).unwrap();
        // let playbin = gst::ElementFactory::make("playbin", None).unwrap();
        // playbin
        //     .set_property("uri", &Value::from("appsrc://"))
        //     .unwrap();

        Player {
            inner: Arc::new(Mutex::new(PlayerInner {
                playbin: playbin,
                appsrc: None,
                eos_reached: false,
                metadata: None,
            })),
        }
    }

    pub fn start(&mut self) {
        // let self_clone = self.clone();
        // let source_handler_id = self.playbin.connect("notify::source", true, move |_| {
        //     let source = self_clone
        //         .playbin
        //         .get_property("source")
        //         .unwrap()
        //         .downcast::<gst::Element>()
        //         .unwrap()
        //         .get()
        //         .unwrap();
        //     let appsrc = source
        //         .clone()
        //         .dynamic_cast::<gst_app::AppSrc>()
        //         .expect("Source element is expected to be an appsrc!");

        //     println!("Got source: {:?}", appsrc);
        //     appsrc.set_property_format(gst::Format::Bytes);
        //     self_clone.set_appsrc(appsrc);
        //     None
        // });

        let inner = self.inner.lock().unwrap();
        let bus = inner.playbin.get_bus().unwrap();

        let ret = inner.playbin.set_state(gst::State::Paused);
        assert_ne!(ret, gst::StateChangeReturn::Failure);

        let inner_clone = self.inner.clone();
        thread::spawn(move || loop {
            let msg = match bus.timed_pop(u64::MAX) {
                None => break,
                Some(msg) => msg,
            };

            match msg.view() {
                gst::MessageView::Eos(..) => {
                    inner_clone.lock().unwrap().handle_eos();
                    break;
                }
                gst::MessageView::Error(err) => {
                    println!(
                        "Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(),
                        err.get_debug().unwrap()
                    );
                    inner_clone.lock().unwrap().stop();
                    break;
                }
                gst::MessageView::StateChanged(s) => match inner_clone.lock() {
                    Ok(mut inner) => if msg.get_src() == inner.playbin {
                        if (s.get_old(), s.get_current()) ==
                            (gst::State::Ready, gst::State::Paused)
                        {
                            inner.check_metadata();
                            println!(
                                "State changed from {}: {:?} -> {:?} ({:?})",
                                msg.get_src().get_path_string(),
                                s.get_old(),
                                s.get_current(),
                                s.get_pending()
                            );

                        }
                    },
                    _ => {}
                },
                _ => (),
            }
        });

    }

    pub fn play(&mut self) {
        let mut inner = self.inner.lock().unwrap();
        if let None = inner.appsrc {
            let source = inner
                .playbin
                .get_property("source")
                .unwrap()
                .downcast::<gst::Element>()
                .unwrap()
                .get()
                .unwrap();
            let appsrc = source
                .clone()
                .dynamic_cast::<gst_app::AppSrc>()
                .expect("Source element is expected to be an appsrc!");

            println!("Got source: {:?}", appsrc);
            appsrc.set_property_format(gst::Format::Bytes);
            appsrc.set_property_block(true);
            inner.appsrc = Some(appsrc);
        }
        inner.playbin.set_state(gst::State::Playing);
    }

    pub fn end_of_stream(&mut self) -> bool {
        self.inner.lock().unwrap().eos_reached
    }

    pub fn stop(&mut self) {
        self.inner.lock().unwrap().stop()
    }

    pub fn get_metadata(&mut self) -> Option<Metadata> {
        match self.inner.lock() {
            Ok(mut i) => i.get_metadata(),
            Err(_) => None,
        }
    }

    pub fn push_data(&mut self, data: &Vec<u8>) -> bool {
        if let Some(ref mut appsrc) = self.inner.lock().unwrap().appsrc {
            let buffer = gst::Buffer::from_vec(data.to_vec()).expect("Unable to create a Buffer");

            if appsrc.push_buffer(buffer) == gst::FlowReturn::Ok {
                return true;
            } else {
                return false;
            }
        } else {
            println!("the stream hasn't been initialized yet");
            return false;
        }
    }
}
