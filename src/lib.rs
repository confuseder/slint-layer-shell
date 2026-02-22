use crate::platform::ProxyTask;
use crate::window_adapter::LayerShellWindowAdapter;
use calloop::{EventLoop, LoopSignal};
use i_slint_renderer_skia::SkiaSharedContext;
use slint::{
    LogicalPosition, PhysicalSize, SharedString,
    platform::{PointerEventButton, WindowEvent},
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_touch, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryHandler, RegistryState},
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{
            BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, PointerEvent, PointerEventKind, PointerHandler,
        },
        touch::TouchHandler,
    },
};
use std::rc::Rc;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Weak,
};
use wayland_backend::client::ObjectId;
use wayland_client::Proxy;
use wayland_client::protocol::{
    wl_keyboard, wl_output::WlOutput, wl_pointer, wl_seat::WlSeat, wl_surface::WlSurface, wl_touch,
};
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init};

mod platform;
mod window_adapter;

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

impl ProvidesRegistryState for LayerShellState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    fn runtime_add_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
        _version: u32,
    ) {
    }

    fn runtime_remove_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
    ) {
    }
}

impl RegistryHandler<LayerShellState> for LayerShellState {
    fn new_global(
        _data: &mut Self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
        _version: u32,
    ) {
    }

    fn remove_global(
        _data: &mut Self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
    ) {
    }
}

impl CompositorHandler for LayerShellState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _time: u32,
    ) {
        let id = surface.id();
        if let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() {
            if let Some(window_adapter) = window_adapter_weak.upgrade() {
                window_adapter.frame_callback_pending.set(false);
                return;
            }
            self.window_adapters.remove(&id);
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        output: &WlOutput,
    ) {
        let id = surface.id();
        let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&id);
            return;
        };

        if let Some(output_info) = self.output_state.info(output) {
            let scale = output_info.scale_factor.max(1) as f32;
            let _ = window_adapter
                .window
                .try_dispatch_event(WindowEvent::ScaleFactorChanged {
                    scale_factor: scale,
                });
            window_adapter.pending_redraw.set(true);
        }
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _output: &WlOutput,
    ) {
        let id = surface.id();
        let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&id);
            return;
        };
        window_adapter.pending_redraw.set(true);
    }
}

impl OutputHandler for LayerShellState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {}

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {}

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {
    }
}

impl SeatHandler for LayerShellState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(err) => eprintln!("failed to create keyboard: {err}"),
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(err) => eprintln!("failed to create pointer: {err}"),
            }
        }
        if capability == Capability::Touch && self.touch.is_none() {
            match self.seat_state.get_touch(qh, &seat) {
                Ok(touch) => self.touch = Some(touch),
                Err(err) => eprintln!("failed to create touch: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
            self.keyboard_focus_surface = None;
        }
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Touch {
            if let Some(touch) = self.touch.take() {
                touch.release();
            }
            self.touch_points.clear();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}
}

impl KeyboardHandler for LayerShellState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        let id = surface.id();
        self.keyboard_focus_surface = Some(id.clone());
        if let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() {
            if let Some(window_adapter) = window_adapter_weak.upgrade() {
                let _ = window_adapter
                    .window
                    .try_dispatch_event(WindowEvent::WindowActiveChanged(true));
                window_adapter.pending_redraw.set(true);
            } else {
                self.window_adapters.remove(&id);
            }
        }
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &WlSurface,
        _serial: u32,
    ) {
        let id = surface.id();
        self.keyboard_focus_surface = None;
        if let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() {
            if let Some(window_adapter) = window_adapter_weak.upgrade() {
                let _ = window_adapter
                    .window
                    .try_dispatch_event(WindowEvent::WindowActiveChanged(false));
                window_adapter.pending_redraw.set(true);
            } else {
                self.window_adapters.remove(&id);
            }
        }
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if let Some((window_adapter, text)) = self
            .keyboard_focus_surface
            .clone()
            .and_then(|id| {
                self.window_adapters
                    .get(&id)
                    .cloned()
                    .and_then(|w| w.upgrade())
            })
            .and_then(|window_adapter| key_event_text(&event).map(|text| (window_adapter, text)))
        {
            let _ = window_adapter
                .window
                .try_dispatch_event(WindowEvent::KeyPressed { text });
            window_adapter.pending_redraw.set(true);
        }
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if let Some((window_adapter, text)) = self
            .keyboard_focus_surface
            .clone()
            .and_then(|id| {
                self.window_adapters
                    .get(&id)
                    .cloned()
                    .and_then(|w| w.upgrade())
            })
            .and_then(|window_adapter| key_event_text(&event).map(|text| (window_adapter, text)))
        {
            let _ = window_adapter
                .window
                .try_dispatch_event(WindowEvent::KeyPressRepeated { text });
            window_adapter.pending_redraw.set(true);
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if let Some((window_adapter, text)) = self
            .keyboard_focus_surface
            .clone()
            .and_then(|id| {
                self.window_adapters
                    .get(&id)
                    .cloned()
                    .and_then(|w| w.upgrade())
            })
            .and_then(|window_adapter| key_event_text(&event).map(|text| (window_adapter, text)))
        {
            let _ = window_adapter
                .window
                .try_dispatch_event(WindowEvent::KeyReleased { text });
            window_adapter.pending_redraw.set(true);
        }
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
    }
}

impl PointerHandler for LayerShellState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let id = event.surface.id();
            let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() else {
                continue;
            };
            let Some(window_adapter) = window_adapter_weak.upgrade() else {
                self.window_adapters.remove(&id);
                continue;
            };

            let position = LogicalPosition::new(event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let _ = window_adapter
                        .window
                        .try_dispatch_event(WindowEvent::PointerMoved { position });
                }
                PointerEventKind::Leave { .. } => {
                    let _ = window_adapter
                        .window
                        .try_dispatch_event(WindowEvent::PointerExited);
                }
                PointerEventKind::Press { button, .. } => {
                    let _ = window_adapter
                        .window
                        .try_dispatch_event(WindowEvent::PointerPressed {
                            position,
                            button: map_pointer_button(button),
                        });
                }
                PointerEventKind::Release { button, .. } => {
                    let _ =
                        window_adapter
                            .window
                            .try_dispatch_event(WindowEvent::PointerReleased {
                                position,
                                button: map_pointer_button(button),
                            });
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    let delta_x = if horizontal.absolute != 0.0 {
                        horizontal.absolute as f32
                    } else {
                        horizontal.discrete as f32 * 15.0
                    };
                    let delta_y = if vertical.absolute != 0.0 {
                        vertical.absolute as f32
                    } else {
                        vertical.discrete as f32 * 15.0
                    };
                    let _ =
                        window_adapter
                            .window
                            .try_dispatch_event(WindowEvent::PointerScrolled {
                                position,
                                delta_x,
                                delta_y,
                            });
                }
            }
            window_adapter.pending_redraw.set(true);
        }
    }
}

impl TouchHandler for LayerShellState {
    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let surface_id = surface.id();
        let Some(window_adapter_weak) = self.window_adapters.get(&surface_id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&surface_id);
            return;
        };

        let position = (position.0 as f32, position.1 as f32);
        self.touch_points.insert(id, (surface_id, position));

        let _ = window_adapter
            .window
            .try_dispatch_event(WindowEvent::PointerPressed {
                position: LogicalPosition::new(position.0, position.1),
                button: PointerEventButton::Left,
            });
        window_adapter.pending_redraw.set(true);
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        let Some((surface_id, position)) = self.touch_points.remove(&id) else {
            return;
        };
        let Some(window_adapter_weak) = self.window_adapters.get(&surface_id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&surface_id);
            return;
        };

        let _ = window_adapter
            .window
            .try_dispatch_event(WindowEvent::PointerReleased {
                position: LogicalPosition::new(position.0, position.1),
                button: PointerEventButton::Left,
            });
        window_adapter.pending_redraw.set(true);
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        let Some((surface_id, _)) = self.touch_points.get(&id).cloned() else {
            return;
        };
        let position = (position.0 as f32, position.1 as f32);
        if let Some((_, stored_position)) = self.touch_points.get_mut(&id) {
            *stored_position = position;
        }

        let Some(window_adapter_weak) = self.window_adapters.get(&surface_id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&surface_id);
            return;
        };

        let _ = window_adapter
            .window
            .try_dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(position.0, position.1),
            });
        window_adapter.pending_redraw.set(true);
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {
        let cancelled = self
            .touch_points
            .drain()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        for (surface_id, position) in cancelled {
            let Some(window_adapter_weak) = self.window_adapters.get(&surface_id).cloned() else {
                continue;
            };
            let Some(window_adapter) = window_adapter_weak.upgrade() else {
                self.window_adapters.remove(&surface_id);
                continue;
            };

            let _ = window_adapter
                .window
                .try_dispatch_event(WindowEvent::PointerReleased {
                    position: LogicalPosition::new(position.0, position.1),
                    button: PointerEventButton::Left,
                });
            window_adapter.pending_redraw.set(true);
        }
    }
}

fn map_pointer_button(button: u32) -> PointerEventButton {
    match button {
        BTN_LEFT => PointerEventButton::Left,
        BTN_RIGHT => PointerEventButton::Right,
        BTN_MIDDLE => PointerEventButton::Middle,
        _ => PointerEventButton::Other,
    }
}

fn key_event_text(event: &KeyEvent) -> Option<SharedString> {
    if let Some(text) = &event.utf8 {
        if !text.is_empty() {
            return Some(text.clone().into());
        }
    }
    event.keysym.key_char().map(Into::into)
}

impl WindowHandler for LayerShellState {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {}

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let id = window.wl_surface().id();
        let Some(window_adapter_weak) = self.window_adapters.get(&id).cloned() else {
            return;
        };
        let Some(window_adapter) = window_adapter_weak.upgrade() else {
            self.window_adapters.remove(&id);
            return;
        };

        let pending_size = window_adapter.pending_size.get();
        let current_size = window_adapter.size.get();
        let fallback_size = pending_size.unwrap_or(current_size);

        let width =
            configure
                .new_size
                .0
                .map(|value| value.get())
                .unwrap_or(if fallback_size.width > 0 {
                    fallback_size.width
                } else {
                    100
                });
        let height =
            configure
                .new_size
                .1
                .map(|value| value.get())
                .unwrap_or(if fallback_size.height > 0 {
                    fallback_size.height
                } else {
                    100
                });

        let size = PhysicalSize::new(width, height);
        window_adapter.size.set(size);
        window_adapter.pending_size.set(None);
        window_adapter
            .window_state
            .set(crate::window_adapter::WindowState::Configured);

        let logical_size = size.to_logical(window_adapter.window.scale_factor());
        let _ = window_adapter
            .window
            .try_dispatch_event(WindowEvent::Resized { size: logical_size });
        window_adapter.pending_redraw.set(true);
    }
}

delegate_registry!(LayerShellState);
delegate_compositor!(LayerShellState);
delegate_output!(LayerShellState);
delegate_seat!(LayerShellState);
delegate_keyboard!(LayerShellState);
delegate_pointer!(LayerShellState);
delegate_touch!(LayerShellState);
delegate_xdg_shell!(LayerShellState);
delegate_xdg_window!(LayerShellState);
