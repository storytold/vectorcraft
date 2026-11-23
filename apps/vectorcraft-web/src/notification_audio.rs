//! Browser synthesis of the same original short tick as the native host.

use std::cell::RefCell;
use std::rc::Rc;
use vectorcraft_ui_egui::toasts::{TICK_ATTACK, TICK_DURATION, TICK_GAIN, TICK_HZ};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{AudioContext, AudioContextState, GainNode, OscillatorNode, OscillatorType};

fn observe(promise: js_sys::Promise) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
            log::warn!("notification audio operation failed: {error:?}");
        }
    });
}

struct Playing {
    oscillator: OscillatorNode,
    gain: GainNode,
    end: f64,
}

impl Playing {
    fn disconnect(&self) {
        // Already stopped sources may refuse another stop; disconnecting still releases the graph.
        self.oscillator.stop().ok();
        self.oscillator.disconnect().ok();
        self.gain.disconnect().ok();
    }
}

#[derive(Default)]
struct Tick {
    context: Option<AudioContext>,
    playing: Vec<Playing>,
}

impl Tick {
    fn activate(&self) -> Result<(), JsValue> {
        if let Some(context) = &self.context
            && context.state() == AudioContextState::Suspended
        {
            // Called in the browser's normal pointer/key event, not from an automation request.
            // No source is queued here, so resuming cannot replay an earlier silent cue.
            // Retry while still suspended: a rejected resume promise must not latch sound off.
            observe(context.resume()?);
        }
        Ok(())
    }

    fn play(&mut self) -> Result<(), JsValue> {
        if self.context.is_none() {
            self.context = Some(AudioContext::new()?);
        }
        let Some(context) = &self.context else { return Ok(()) };
        if context.state() != AudioContextState::Running {
            // The next normal user activation can resume the context. This cue is not delayed
            // until then: scheduling here would replay stale sounds after activation.
            return Ok(());
        }
        let now = context.current_time();
        self.playing.retain(|sound| {
            if sound.end > now {
                true
            } else {
                sound.disconnect();
                false
            }
        });
        if self.playing.len() == 32 {
            return Ok(());
        }
        let oscillator = context.create_oscillator()?;
        let gain = context.create_gain()?;
        oscillator.set_type(OscillatorType::Sine);
        oscillator.frequency().set_value(TICK_HZ as f32);
        let result = (|| {
            gain.gain().set_value_at_time(0.0, now)?;
            gain.gain().linear_ramp_to_value_at_time(TICK_GAIN as f32, now + TICK_ATTACK)?;
            gain.gain().linear_ramp_to_value_at_time(0.0, now + TICK_DURATION)?;
            oscillator.connect_with_audio_node(&gain)?;
            gain.connect_with_audio_node(&context.destination())?;
            oscillator.start_with_when(now)?;
            oscillator.stop_with_when(now + TICK_DURATION)?;
            Ok(())
        })();
        let sound = Playing { oscillator, gain, end: now + TICK_DURATION };
        if let Err(error) = result {
            sound.disconnect();
            return Err(error);
        }
        self.playing.push(sound);
        Ok(())
    }
}

impl Drop for Tick {
    fn drop(&mut self) {
        for sound in &self.playing {
            sound.disconnect();
        }
        if let Some(context) = &self.context
            && let Ok(promise) = context.close()
        {
            observe(promise);
        }
    }
}

struct Service {
    tick: Rc<RefCell<Tick>>,
    activation: Closure<dyn FnMut(web_sys::Event)>,
    window: Option<web_sys::Window>,
}

impl Service {
    fn play(&self) {
        if let Ok(mut tick) = self.tick.try_borrow_mut()
            && let Err(error) = tick.play()
        {
            log::warn!("notification sound is unavailable: {error:?}");
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some(window) = &self.window {
            for event in ["pointerdown", "keydown"] {
                window.remove_event_listener_with_callback_and_bool(event, self.activation.as_ref().unchecked_ref(), true).ok();
            }
        }
    }
}

pub(crate) fn start() -> Box<dyn FnMut()> {
    let tick = Rc::new(RefCell::new(Tick::default()));
    let activated = tick.clone();
    let activation: Closure<dyn FnMut(web_sys::Event)> = Closure::new(move |event: web_sys::Event| {
        if event.is_trusted()
            && let Ok(tick) = activated.try_borrow()
            && let Err(error) = tick.activate()
        {
            log::warn!("could not resume notification sound: {error:?}");
        }
    });
    let window = web_sys::window();
    if let Some(window) = &window {
        for event in ["pointerdown", "keydown"] {
            if let Err(error) = window.add_event_listener_with_callback_and_bool(event, activation.as_ref().unchecked_ref(), true) {
                log::warn!("could not observe notification sound activation: {error:?}");
            }
        }
    }
    let service = Service { tick, activation, window };
    Box::new(move || service.play())
}
