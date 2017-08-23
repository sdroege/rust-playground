extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;
extern crate gstreamer_player as gst_player;
extern crate glib;
use self::glib::*;

use std::u64;
use std::time;
use std::string;
use std::sync::{Arc, Mutex};

use self::gst_player::PlayerMediaInfoExt;
use self::gst_player::PlayerVideoInfoExt;
use self::gst_player::PlayerStreamInfoExt;

struct PlayerInner<E> {
    player: gst_player::Player,
    appsrc: Option<gst_app::AppSrc>,
    input_size: u64,
    subscribers: Vec<Box<Fn(&E) + Send>>,
}

#[derive(Debug)]
pub struct Metadata {
    pub duration: Option<time::Duration>,
    pub width: u32,
    pub height: u32,
    pub format: string::String,
    // TODO: Might be nice to move width and height along with each video track.
    pub video_tracks: Vec<string::String>,
    pub audio_tracks: Vec<string::String>,
}


#[derive(Debug)]
pub enum PlayerEvent {
    EndOfStream,
    MetadataUpdated(Metadata),
}

pub struct Player {
    inner: Arc<Mutex<PlayerInner<PlayerEvent>>>,
}

impl<E> PlayerInner<E> {
    pub fn register_event_handler<F>(&mut self, callback: F)
    where
        F: 'static + Fn(&E) + Send,
    {
        self.subscribers.push(Box::new(callback));
    }

    pub fn notify(&self, event: E) {
        for callback in &self.subscribers {
            callback(&event);
        }
    }

    pub fn set_input_size(&mut self, size: u64) {
        self.input_size = size;
    }

    pub fn play(&mut self) {
        self.player.play();
    }

    pub fn stop(&mut self) {
        self.player.stop();
    }

    pub fn start(&mut self) {
        self.player.pause();
    }

    pub fn set_app_src(&mut self, appsrc: gst_app::AppSrc) {
        self.appsrc = Some(appsrc);
    }

    pub fn get_metadata(&self, media_info: &PlayerMediaInfoExt) -> Metadata {
        let dur = media_info.get_duration();
        let mut duration = None;
        if dur != u64::MAX {
            let secs = dur / 1_000_000_000;
            let nanos = dur % 1_000_000_000;
            duration = Some(time::Duration::new(secs, nanos as u32));
        }

        let mut format = string::String::from("");
        let mut audio_tracks = Vec::new();
        let mut video_tracks = Vec::new();
        if let Some(f) = media_info.get_container_format() {
            format = f;
        }

        for stream_info in media_info.get_stream_list() {
            if let Some(stream_type) = stream_info.get_stream_type() {
                match stream_type.as_str() {
                    "audio" => {
                        audio_tracks.push(stream_info.get_codec().unwrap());
                    }
                    "video" => {
                        video_tracks.push(stream_info.get_codec().unwrap());
                    }
                    _ => {}
                }
            }
        }

        let mut width = 0;
        let mut height = 0;
        if media_info.get_number_of_video_streams() > 0 {
            let first_video_stream = &media_info.get_video_streams()[0];
            width = first_video_stream.get_width();
            height = first_video_stream.get_height();
        }
        Metadata {
            duration: duration,
            width: width as u32,
            height: height as u32,
            format: format,
            audio_tracks: audio_tracks,
            video_tracks: video_tracks,
        }

    }
}


impl Player {
    pub fn new() -> Player {
        let player = gst_player::Player::new(None, None);
        player
            .set_property("uri", &Value::from("appsrc://"))
            .expect("Can't set uri property");

        // Disable periodic position updates for now.
        let config = gst::Structure::new("config", &[("position-interval-update", &0u32)]);
        player.set_config(config);

        // FIXME: glimagesink can't be used because:
        // 1. test-player isn't a Cocoa app running a NSApplication
        // 2. the GstGLDisplayCocoa depends on a main GLib loop in that case ^^ which test-player isn't using
        let pipeline = player.get_pipeline().unwrap();
        if let Some(sink) = gst::ElementFactory::make("osxvideosink", None) {
            pipeline
                .set_property("video-sink", &sink.to_value())
                .expect("Can't set video sink property");
        }

        Player {
            inner: Arc::new(Mutex::new(PlayerInner::<PlayerEvent> {
                player: player,
                appsrc: None,
                input_size: 0,
                subscribers: Vec::new(),
            })),
        }
    }

    pub fn register_event_handler<F>(&mut self, callback: F)
    where
        F: 'static + Fn(&PlayerEvent) + Send,
    {
        self.inner.lock().unwrap().register_event_handler(callback);
    }

    pub fn set_input_size(&mut self, size: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.set_input_size(size);
        }
    }

    pub fn start(&mut self) {
        let inner_clone = self.inner.clone();
        self.inner.lock().unwrap().player.connect_end_of_stream(
            move |_| {
                let inner = Arc::clone(&inner_clone);
                let guard = inner.lock().unwrap();

                guard.notify(PlayerEvent::EndOfStream);
            },
        );

        self.inner.lock().unwrap().player.connect_state_changed(
            move |_, state| {
                println!("new state: {:?}", state);
            },
        );

        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .player
            .connect_media_info_updated(move |_, info| {
                let inner = Arc::clone(&inner_clone);
                let guard = inner.lock().unwrap();

                guard.notify(PlayerEvent::MetadataUpdated(guard.get_metadata(info)));

            });

        self.inner.lock().unwrap().player.connect_duration_changed(
            move |_, duration| {
                let mut seconds = duration / 1_000_000_000;
                let mut minutes = seconds / 60;
                let hours = minutes / 60;

                seconds %= 60;
                minutes %= 60;

                println!(
                    "Duration changed to: {:02}:{:02}:{:02}",
                    hours,
                    minutes,
                    seconds
                );
            },
        );

        if let Ok(mut inner) = self.inner.lock() {
            inner.start();
        }
    }

    pub fn ready(&mut self) -> bool {
        if let Ok(mut inner) = self.inner.lock() {
            if let None = inner.appsrc {
                let pipeline = inner.player.get_pipeline().unwrap();
                let source = pipeline.get_property("source").unwrap();
                if let Some(source) = source.downcast::<gst::Element>().unwrap().get() {
                    let appsrc = source
                        .clone()
                        .dynamic_cast::<gst_app::AppSrc>()
                        .expect("Source element is expected to be an appsrc!");

                    println!("Got source: {:?}", appsrc);
                    appsrc.set_property_format(gst::Format::Bytes);
                    appsrc.set_property_block(true);
                    appsrc.set_size(inner.input_size as i64);
                    inner.set_app_src(appsrc);
                    true
                } else {
                    false
                }
            } else {
                true
            }
        } else {
            false
        }
    }

    pub fn play(&mut self) {
        self.inner.lock().unwrap().play();
    }

    pub fn stop(&mut self) {
        self.inner.lock().unwrap().stop();
    }

    pub fn push_data(&mut self, data: &[u8]) -> bool {
        if let Some(ref mut appsrc) = self.inner.lock().unwrap().appsrc {
            let v = Vec::from(data);
            let buffer = gst::Buffer::from_vec(v).expect("Unable to create a Buffer");

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
