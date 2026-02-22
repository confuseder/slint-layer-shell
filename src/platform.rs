use crate::{SlintLayerShell, window_adapter::LayerShellWindowAdapter};
use calloop::LoopSignal;
use i_slint_core::api::EventLoopError;
use i_slint_core::platform::{EventLoopProxy, update_timers_and_animations};
use slint::platform::{Platform, PlatformError, WindowAdapter, duration_until_next_timer_update};
use std::rc::Rc;
use std::time::Instant;

impl Platform for SlintLayerShell {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let qh = self.queue_handle.clone();

        let surface = {
            let state = self.state.borrow_mut();
            state.compositor_state.create_surface(&qh)
        };

        match LayerShellWindowAdapter::new(surface, self.connection.clone(), self.state.clone(), qh)
        {
            Ok(adapter) => Ok(adapter),
            Err(e) => Err(e),
        }
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let mut fps_frame_count: u128 = 0;
        let mut fps_window_start = Instant::now();

        loop {
            if self.should_close {
                self.loop_signal.stop();
                break;
            }

            let mut state = self.state.borrow_mut();
            let mut event_loop = self.event_loop.borrow_mut();

            while let Some(task) = state.proxied_event_queue.pop_front() {
                task();
            }

            // Update slint's animate timer.
            update_timers_and_animations();

            // TODO: Execute invoke function from channel.
            state.window_adapters.values().for_each(|window_adapter| {
                let Some(window_adapter) = window_adapter.upgrade() else {
                    return;
                };

                if window_adapter.window_state.get()
                    != crate::window_adapter::WindowState::Configured
                {
                    return;
                }

                if window_adapter.frame_callback_pending.get() {
                    return;
                }

                if window_adapter.pending_redraw.get() {
                    // {
                    fps_frame_count += 1;

                    let elapsed = fps_window_start.elapsed();
                    if elapsed.as_secs_f64() >= 1.0 {
                        let fps = fps_frame_count as f64 / elapsed.as_secs_f64();
                        println!("FPS: {:.2}", fps);
                        fps_frame_count = 0;
                        fps_window_start = Instant::now();
                    }

                    window_adapter
                        .surface
                        .frame(&self.queue_handle, window_adapter.surface.clone());
                    let _ = window_adapter.render.render();
                    window_adapter.frame_callback_pending.set(true);
                    window_adapter.pending_redraw.set(false);
                }
            });

            // println!("Duration: {:?}", duration_until_next_timer_update());
            let _ = event_loop.dispatch(duration_until_next_timer_update(), &mut state);
        }

        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        let (event_loop_proxy, rx) = LayerShellEventLoopProxy::new(self.loop_signal.clone());

        let _ = self
            .event_loop
            .borrow_mut()
            .handle()
            .insert_source(rx, |event, _, state| {
                if let calloop::channel::Event::Msg(task) = event {
                    state.proxied_event_queue.push_back(task);
                }
            });

        Some(Box::new(event_loop_proxy))
    }
}

pub type ProxyTask = Box<dyn FnOnce() + Send>;

struct LayerShellEventLoopProxy {
    loop_signal: LoopSignal,
    tx: calloop::channel::Sender<ProxyTask>,
}

impl LayerShellEventLoopProxy {
    fn new(loop_signal: LoopSignal) -> (Self, calloop::channel::Channel<ProxyTask>) {
        let (tx, rx) = calloop::channel::channel();

        (Self { loop_signal, tx }, rx)
    }
}

impl EventLoopProxy for LayerShellEventLoopProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.loop_signal.stop();
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.tx
            .send(event)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}
