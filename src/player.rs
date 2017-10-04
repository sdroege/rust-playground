extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;
extern crate gstreamer_player as gst_player;
use self::glib::*;

extern crate ipc_channel;
use self::ipc_channel::ipc;

use std::u64;
use std::time;
use std::string;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

use self::gst_player::PlayerMediaInfo;
use self::gst_player::PlayerStreamInfoExt;

struct PlayerInner {
    player: gst_player::Player,
    appsrc: Option<gst_app::AppSrc>,
    appsink: gst_app::AppSink,
    input_size: u64,
    subscribers: Vec<ipc::IpcSender<PlayerEvent>>,
    renderers: Vec<Box<FrameRenderer>>,
    last_metadata: Option<Metadata>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub duration: Option<time::Duration>,
    pub width: u32,
    pub height: u32,
    pub format: string::String,
    // TODO: Might be nice to move width and height along with each video track.
    pub video_tracks: Vec<string::String>,
    pub audio_tracks: Vec<string::String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PlaybackState {
    Stopped,
    // Buffering,
    Paused,
    Playing,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PlayerEvent {
    EndOfStream,
    MetadataUpdated(Metadata),
    StateChanged(PlaybackState),
    FrameUpdated,
    Error,
}

#[derive(Clone)]
pub struct Frame {
    width: i32,
    height: i32,
    // TODO:
    // buffer: gst::Buffer

    // width * height * 4
    data: Arc<Vec<u8>>,
}

impl Frame {
    fn new(sample: &gst::Sample) -> Frame {
        let caps = sample.get_caps().unwrap();
        let s = caps.get_structure(0).unwrap();
        let width = s.get("width").unwrap();
        let height = s.get("height").unwrap();
        let buffer = sample.get_buffer().unwrap();

        let map = buffer.map_readable().unwrap();
        let data = Vec::from(map.as_slice());

        Frame {
            width: width,
            height: height,
            data: Arc::new(data),
        }
    }

    pub fn get_width(&self) -> i32 {
        self.width
    }

    pub fn get_height(&self) -> i32 {
        self.height
    }

    pub fn get_data(&self) -> &Arc<Vec<u8>> {
        &self.data
    }
}

pub trait FrameRenderer: Send + Sync + 'static {
    fn render(&self, frame: Frame);
}

#[derive(Clone)]
pub struct Player {
    inner: Arc<Mutex<PlayerInner>>,
}

impl PlayerInner {
    pub fn register_event_handler(&mut self, sender: ipc::IpcSender<PlayerEvent>) {
        self.subscribers.push(sender);
    }

    pub fn register_frame_renderer(&mut self, renderer: Box<FrameRenderer>) {
        self.renderers.push(renderer);
    }

    pub fn notify(&self, event: PlayerEvent) {
        for sender in &self.subscribers {
            sender.send(event.clone()).unwrap();
        }
    }

    pub fn render(&self, sample: &gst::Sample) {
        let frame = Frame::new(&sample);

        for renderer in &self.renderers {
            renderer.render(frame.clone());
        }
        self.notify(PlayerEvent::FrameUpdated);
    }

    pub fn set_input_size(&mut self, size: u64) {
        self.input_size = size;
    }

    pub fn play(&mut self) {
        self.player.play();
    }

    pub fn stop(&mut self) {
        self.player.stop();
        self.last_metadata = None;
        self.appsrc = None;
    }

    pub fn start(&mut self) {
        self.player.pause();
    }

    pub fn set_app_src(&mut self, appsrc: gst_app::AppSrc) {
        self.appsrc = Some(appsrc);
    }
}

pub fn media_info_to_metadata(media_info: &PlayerMediaInfo) -> Metadata {
    let dur = media_info.get_duration();
    let duration = if dur != u64::MAX {
        let secs = dur / 1_000_000_000;
        let nanos = dur % 1_000_000_000;

        Some(time::Duration::new(secs, nanos as u32))
    } else {
        None
    };

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
    let height = if media_info.get_number_of_video_streams() > 0 {
        let first_video_stream = &media_info.get_video_streams()[0];
        width = first_video_stream.get_width();
        first_video_stream.get_height()
    } else {
        0
    };
    Metadata {
        duration: duration,
        width: width as u32,
        height: height as u32,
        format: format,
        audio_tracks: audio_tracks,
        video_tracks: video_tracks,
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

        /*
        #[cfg(target_os = "macos")]
        {
            // FIXME: glimagesink can't be used because:
            // 1. test-player isn't a Cocoa app running a NSApplication
            // 2. the GstGLDisplayCocoa depends on a main GLib loop in that case ^^ which test-player
            // is not using
            let pipeline = player.get_pipeline().unwrap();
            if let Some(sink) = gst::ElementFactory::make("osxvideosink", None) {
                pipeline
                    .set_property("video-sink", &sink.to_value())
                    .expect("Can't set video sink property");
            }
        }
        */
        let video_sink = gst::ElementFactory::make("appsink", None).unwrap();
        let pipeline = player.get_pipeline().unwrap();
        pipeline.set_property("video-sink", &video_sink).unwrap();
        let video_sink = video_sink.dynamic_cast::<gst_app::AppSink>().unwrap();
        video_sink.set_caps(&gst::Caps::new_simple(
            "video/x-raw",
            &[
                ("format", &"BGRA"),
                ("pixel-aspect-ratio", &gst::Fraction::from((1, 1))),
            ]
        ));

        Player {
            inner: Arc::new(Mutex::new(PlayerInner {
                player: player,
                appsrc: None,
                appsink: video_sink,
                input_size: 0,
                subscribers: Vec::new(),
                renderers: Vec::new(),
                last_metadata: None,
            })),
        }
    }

    pub fn register_event_handler(&self, sender: ipc::IpcSender<PlayerEvent>)
    {
        self.inner.lock().unwrap().register_event_handler(sender);
    }

    pub fn register_frame_renderer<R: FrameRenderer>(&self, renderer: R) {
        self.inner.lock().unwrap().register_frame_renderer(Box::new(renderer));
    }

    pub fn set_input_size(&self, size: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.set_input_size(size);
        }
    }

    pub fn start(&self) -> bool {
        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .player
            .connect_end_of_stream(move |_| {
                let inner = &inner_clone;
                let guard = inner.lock().unwrap();

                guard.notify(PlayerEvent::EndOfStream);
            });

        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .player
            .connect_error(move |_, _| {
                let inner = &inner_clone;
                let guard = inner.lock().unwrap();

                guard.notify(PlayerEvent::Error);
            });

        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .player
            .connect_state_changed(move |_, player_state| {
                let state = match player_state {
                    gst_player::PlayerState::Stopped => Some(PlaybackState::Stopped),
                    gst_player::PlayerState::Paused => Some(PlaybackState::Paused),
                    gst_player::PlayerState::Playing => Some(PlaybackState::Playing),
                    _ => None,
                };
                if let Some(v) = state {
                    let inner = &inner_clone;
                    let guard = inner.lock().unwrap();

                    guard.notify(PlayerEvent::StateChanged(v));
                }
            });

        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .player
            .connect_media_info_updated(move |_, info| {
                let inner = &inner_clone;
                let mut guard = inner.lock().unwrap();

                let metadata = media_info_to_metadata(info);
                if guard.last_metadata.as_ref() != Some(&metadata) {
                    guard.last_metadata = Some(metadata.clone());
                    guard.notify(PlayerEvent::MetadataUpdated(metadata));
                }
            });

        self.inner
            .lock()
            .unwrap()
            .player
            .connect_duration_changed(move |_, duration| {
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
            });

        let inner_clone = self.inner.clone();
        self.inner
            .lock()
            .unwrap()
            .appsink
            .set_callbacks(gst_app::AppSinkCallbacks::new(
            /* eos */
            |_| {},
            /* new_preroll */
            |_| gst::FlowReturn::Ok,
            /* new_samples */
            move |appsink| {
                let sample = match appsink.pull_sample() {
                    None => return gst::FlowReturn::Eos,
                    Some(sample) => sample,
                };

                inner_clone.lock().unwrap().render(&sample);

                gst::FlowReturn::Ok
            }
        ));

        let inner_clone = self.inner.clone();
        let (receiver, error_id) = {
            let mut inner = self.inner.lock().unwrap();
            let pipeline = inner.player.get_pipeline().unwrap();

            let (sender, receiver) = mpsc::channel();

            let sender = Arc::new(Mutex::new(sender));
            let sender_clone = sender.clone();
            pipeline
                .connect("source-setup", false, move |args| {
                    let mut inner = inner_clone.lock().unwrap();

                    if let Some(source) = args[1].get::<gst::Element>() {
                        let appsrc = source
                            .clone()
                            .dynamic_cast::<gst_app::AppSrc>()
                            .expect("Source element is expected to be an appsrc!");

                        appsrc.set_property_format(gst::Format::Bytes);
                        // appsrc.set_property_block(true);
                        if inner.input_size > 0 {
                            appsrc.set_size(inner.input_size as i64);
                        }

                        let sender_clone = sender.clone();

                        let need_data_id = Arc::new(Mutex::new(None));
                        let need_data_id_clone = need_data_id.clone();
                        appsrc.connect("need-data", false, move |args| {
                            let _ = sender_clone.lock().unwrap().send(Ok(()));
                            if let Some(id) = need_data_id_clone.lock().unwrap().take() {
                                glib::signal::signal_handler_disconnect(
                                    &args[0].get::<gst::Element>().unwrap(),
                                    id,
                                );
                            }
                            None
                        }).unwrap();

                        inner.set_app_src(appsrc);
                    } else {
                        let _ = sender.lock().unwrap().send(Err(()));
                    }

                    None
                })
                .unwrap();

            let error_id = inner.player.connect_error(move |_, _| {
                let _ = sender_clone.lock().unwrap().send(Err(()));
            });

            inner.start();

            (receiver, error_id)
        };

        let res = match receiver.recv().unwrap() {
            Ok(_) => {
                true
            },
            Err(_) => {
                false
            },
        };

        glib::signal::signal_handler_disconnect(&self.inner.lock().unwrap().player, error_id);

        res
    }

    pub fn play(&self) {
        self.inner.lock().unwrap().play();
    }

    pub fn stop(&self) {
        self.inner.lock().unwrap().stop();
    }

    pub fn push_data(&self, data: Vec<u8>) -> bool {
        if let Some(ref mut appsrc) = self.inner.lock().unwrap().appsrc {
            let buffer = gst::Buffer::from_vec(data).expect("Unable to create a Buffer");
            return appsrc.push_buffer(buffer) == gst::FlowReturn::Ok;
        } else {
            println!("the stream hasn't been initialized yet");
            return false;
        }
    }

    pub fn end_of_stream(&self) -> bool {
        if let Some(ref mut appsrc) = self.inner.lock().unwrap().appsrc {
            return appsrc.end_of_stream() == gst::FlowReturn::Ok;
        } else {
            println!("the stream hasn't been initialized yet");
            return false;
        }
    }
}
