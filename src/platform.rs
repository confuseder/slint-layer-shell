use crate::window_adapter::LayerShellWindowAdapter;
use calloop::{EventLoop, LoopSignal};
use i_slint_core::api::EventLoopError;
use i_slint_core::platform::{EventLoopProxy, update_timers_and_animations};
use i_slint_renderer_skia::SkiaSharedContext;
use slint::platform::{Platform, PlatformError, WindowAdapter, duration_until_next_timer_update};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::XdgShell;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Instant;
use wayland_backend::client::ObjectId;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_pointer, wl_touch};
use wayland_client::{Connection, QueueHandle};

pub struct LayerShellState {
    pub registry_state: RegistryState,
    pub compositor_state: CompositorState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    // pub layer_shell: LayerShell,
    pub xdg_shell: XdgShell,

    pub skia_shard_context: SkiaSharedContext,

    pub proxied_event_queue: VecDeque<ProxyTask>,

    pub window_adapters: HashMap<ObjectId, Weak<LayerShellWindowAdapter>>,
    pub window_factory_queue: VecDeque<LayerShellWindowAdapter>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub touch: Option<wl_touch::WlTouch>,
    pub keyboard_focus_surface: Option<ObjectId>,
    pub touch_points: HashMap<i32, (ObjectId, (f32, f32))>,
}

pub struct SlintLayerShell {
    connection: Connection,
    // event_queue: EventQueue<LayerShellState>,
    queue_handle: QueueHandle<LayerShellState>,
    state: Rc<RefCell<LayerShellState>>,
    event_loop: RefCell<EventLoop<'static, LayerShellState>>,
    loop_signal: LoopSignal,

    should_close: bool,
}

impl SlintLayerShell {
    pub fn new() -> Self {
        let event_loop = EventLoop::try_new().unwrap();
        let loop_signal = event_loop.get_signal();

        let connection = Connection::connect_to_env().unwrap();
        let (global, event_queue) = registry_queue_init(&connection).unwrap();
        let qh = event_queue.handle();

        let event_source = WaylandSource::<LayerShellState>::new(connection.clone(), event_queue);

        let _ = event_loop
            .handle()
            .insert_source(event_source, |_, queue, state| {
                queue.dispatch_pending(state)
            });

        let registry_state = RegistryState::new(&global);
        let compositor_state = CompositorState::bind(&global, &qh).unwrap();
        let seat_state = SeatState::new(&global, &qh);
        let output_state = OutputState::new(&global, &qh);
        // let layer_shell = LayerShell::bind(&global, &qh).unwrap();
        let xdg_shell = XdgShell::bind(&global, &qh).unwrap();

        let skia_shard_context = SkiaSharedContext::default();

        let state = LayerShellState {
            registry_state,
            compositor_state,
            seat_state,
            output_state,
            // layer_shell,
            xdg_shell,

            skia_shard_context,

            proxied_event_queue: VecDeque::new(),

            window_adapters: HashMap::new(),
            window_factory_queue: VecDeque::new(),
            keyboard: None,
            pointer: None,
            touch: None,
            keyboard_focus_surface: None,
            touch_points: HashMap::new(),
        };

        Self {
            connection,
            queue_handle: qh,
            // event_queue: RefCell::new(event_queue),
            state: Rc::new(RefCell::new(state)),
            event_loop: RefCell::new(event_loop),
            loop_signal,
            should_close: false,
        }
    }
}

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
