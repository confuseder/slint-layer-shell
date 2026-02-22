use crate::platform::LayerShellState;
use i_slint_renderer_skia::SkiaRenderer;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use slint::{
    PhysicalSize, Window as SlintWindow,
    platform::{PlatformError, WindowAdapter},
};
use smithay_client_toolkit::shell::{
    WaylandSurface, wlr_layer::LayerSurface, xdg::window::Window as XdgWindow,
    xdg::window::WindowDecorations,
};
use std::cell::RefCell;
use std::fmt;
use std::{cell::Cell, ptr::NonNull, rc::Rc, sync::Arc};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WindowState {
    Pending,
    Configured,
    Destroy,
}

pub struct LayerShellWindowAdapter {
    pub layer_shell_state: Rc<RefCell<LayerShellState>>,

    pub render: SkiaRenderer,

    pub window: SlintWindow,
    pub surface: WlSurface,
    pub xdg_window: Option<XdgWindow>,
    pub layer_surface: Option<LayerSurface>,
    pub connection: Connection,

    pub window_state: Cell<WindowState>,
    pub pending_redraw: Cell<bool>,
    pub frame_callback_pending: Cell<bool>,
    pub size: Cell<PhysicalSize>,
    pub pending_size: Cell<Option<PhysicalSize>>,
}

struct HandleHelper {
    surface: WlSurface,
    connection: Connection,
}

impl HasWindowHandle for HandleHelper {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle =
            WaylandWindowHandle::new(NonNull::new(self.surface.id().as_ptr() as *mut _).unwrap());
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle))) }
    }
}

impl HasDisplayHandle for HandleHelper {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let handle = WaylandDisplayHandle::new(
            NonNull::new(self.connection.backend().display_ptr() as *mut _).unwrap(),
        );
        unsafe { Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle))) }
    }
}

impl LayerShellWindowAdapter {
    pub fn new(
        surface: WlSurface,
        connection: Connection,
        layer_shell_state: Rc<RefCell<LayerShellState>>,
        qh: QueueHandle<LayerShellState>,
    ) -> Result<Rc<Self>, PlatformError> {
        let skia_context = layer_shell_state.borrow().skia_shard_context.clone();
        let handle_helper = Arc::new(HandleHelper {
            surface: surface.clone(),
            connection: connection.clone(),
        });
        let render = SkiaRenderer::default_wgpu_27(&skia_context);
        render.set_window_handle(
            handle_helper.clone(),
            handle_helper.clone(),
            PhysicalSize::new(120, 120),
            None,
        )?;

        let xdg_window = {
            let state = layer_shell_state.borrow();
            state
                .xdg_shell
                .create_window(surface.clone(), WindowDecorations::RequestServer, &qh)
        };
        xdg_window.set_title("slint-layer-shell");
        xdg_window.set_app_id("slint-layer-shell");
        xdg_window.commit();

        let adapter = Rc::new_cyclic(|weak_self: &std::rc::Weak<Self>| {
            let weak_dyn: std::rc::Weak<dyn WindowAdapter> = weak_self.clone();
            let window = SlintWindow::new(weak_dyn);

            Self {
                layer_shell_state: layer_shell_state.clone(),
                render,
                window,
                surface: surface.clone(),
                xdg_window: Some(xdg_window.clone()),
                layer_surface: None,
                connection: connection.clone(),

                window_state: Cell::new(WindowState::Pending),
                pending_redraw: Cell::new(false),
                frame_callback_pending: Cell::new(false),
                size: Cell::new(PhysicalSize::new(0, 0)),
                pending_size: Cell::new(None),
            }
        });

        let id = adapter.surface.id();
        layer_shell_state
            .borrow_mut()
            .window_adapters
            .insert(id, Rc::downgrade(&adapter));

        Ok(adapter)
    }

    pub fn set_size(&self, size: PhysicalSize) {
        self.pending_size.set(Some(size));
        self.pending_redraw.set(true);
    }

    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }
}

impl WindowAdapter for LayerShellWindowAdapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn set_visible(&self, visible: bool) -> Result<(), PlatformError> {
        if !visible {
            self.surface.attach(None::<&WlBuffer>, 0, 0);
            self.surface.commit();
        }
        Ok(())
    }

    fn size(&self) -> slint::PhysicalSize {
        self.size.get()
    }

    fn request_redraw(&self) {
        self.pending_redraw.set(true);
    }

    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.render
    }

    fn update_window_properties(&self, properties: slint::platform::WindowProperties<'_>) {
        println!("{:#?}", DebugWindowProperties(properties));
    }
}

struct DebugWindowProperties<'a>(slint::platform::WindowProperties<'a>);

impl fmt::Debug for DebugWindowProperties<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let props = &self.0;
        f.debug_struct("WindowProperties")
            .field("title", &props.title())
            .field("layout_constraints", &props.layout_constraints())
            .field("is_fullscreen", &props.is_fullscreen())
            .field("is_maximized", &props.is_maximized())
            .field("is_minimized", &props.is_minimized())
            .finish()
    }
}
