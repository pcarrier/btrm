use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufMappingMode};
use smithay::backend::allocator::{Buffer, Fourcc, Modifier, Format as DmabufFormat};
use smithay::backend::input::{Axis, ButtonState, KeyState};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::delegate_compositor;
use smithay::delegate_cursor_shape;
use smithay::delegate_data_device;
use smithay::delegate_dmabuf;
use smithay::delegate_fractional_scale;
use smithay::delegate_output;
use smithay::delegate_text_input_manager;
use smithay::delegate_primary_selection;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_viewporter;
use smithay::delegate_xdg_activation;
use smithay::delegate_xdg_decoration;
use smithay::delegate_xdg_shell;
use smithay::delegate_xdg_toplevel_icon;
use smithay::desktop::{Space, Window};
use smithay::input::keyboard::{FilterResult, XkbConfig};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopSignal, PostAction};
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
use smithay::utils::{Serial, Transform, SERIAL_COUNTER};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    self, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
    with_states, with_surface_tree_downward, TraversalAction,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shm::{BufferData, ShmHandler, ShmState, with_buffer_contents};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::fractional_scale::{FractionalScaleHandler, FractionalScaleManagerState};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconHandler;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;

pub enum CompositorEvent {
    SurfaceCreated {
        surface_id: u16,
        title: String,
        app_id: String,
        parent_id: u16,
        width: u16,
        height: u16,
    },
    SurfaceDestroyed {
        surface_id: u16,
    },
    SurfaceCommit {
        surface_id: u16,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    SurfaceTitle {
        surface_id: u16,
        title: String,
    },
    SurfaceAppId {
        surface_id: u16,
        app_id: String,
    },
    SurfaceResized {
        surface_id: u16,
        width: u16,
        height: u16,
    },
    ClipboardContent {
        surface_id: u16,
        mime_type: String,
        data: Vec<u8>,
    },
}

pub enum CompositorCommand {
    KeyInput {
        surface_id: u16,
        keycode: u32,
        pressed: bool,
    },
    PointerMotion {
        surface_id: u16,
        x: f64,
        y: f64,
    },
    PointerButton {
        surface_id: u16,
        button: u32,
        pressed: bool,
    },
    PointerAxis {
        surface_id: u16,
        axis: u8,
        value: f64,
    },
    SurfaceResize {
        surface_id: u16,
        width: u16,
        height: u16,
        /// DPR in 1/120th units (Wayland convention): 120 = 1×, 240 = 2×.  0 = unchanged.
        scale_120: u16,
    },
    SurfaceFocus {
        surface_id: u16,
    },
    ClipboardOffer {
        surface_id: u16,
        mime_type: String,
        data: Vec<u8>,
    },
    Capture {
        surface_id: u16,
        reply: mpsc::SyncSender<Option<(u32, u32, Vec<u8>)>>,
    },
    /// Fire pending wl_surface.frame callbacks for a surface so the
    /// client will paint and commit its next frame.  Send this when
    /// the server is ready to consume a new frame (streaming or capture).
    RequestFrame {
        surface_id: u16,
    },
    Shutdown,
}

struct SurfaceInfo {
    surface_id: u16,
    window: Window,
    last_width: u32,
    last_height: u32,
    last_title: String,
    last_app_id: String,
}

struct ClientData {
    compositor_state: CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for ClientData {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}

pub struct Compositor {
    display_handle: DisplayHandle,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    #[allow(dead_code)]
    viewporter_state: ViewporterState,
    #[allow(dead_code)]
    xdg_decoration_state: XdgDecorationState,
    dmabuf_state: DmabufState,
    #[allow(dead_code)]
    dmabuf_global: DmabufGlobal,
    primary_selection_state: PrimarySelectionState,
    activation_state: XdgActivationState,
    seat: Seat<Self>,
    #[allow(dead_code)]
    output: Output,
    space: Space<Window>,

    surfaces: HashMap<u64, SurfaceInfo>,
    surface_lookup: HashMap<u16, u64>,
    next_surface_id: u16,

    event_tx: mpsc::Sender<CompositorEvent>,
    event_notify: Arc<dyn Fn() + Send + Sync>,
    loop_signal: LoopSignal,

    #[allow(dead_code)]
    renderer: PixmanRenderer,

    verbose: bool,
}

impl Compositor {
    fn allocate_surface_id(&mut self) -> u16 {
        let id = self.next_surface_id;
        self.next_surface_id = self.next_surface_id.wrapping_add(1);
        if self.next_surface_id == 0 {
            self.next_surface_id = 1;
        }
        id
    }

    fn handle_command(&mut self, cmd: CompositorCommand) {
        match cmd {
            CompositorCommand::KeyInput {
                surface_id,
                keycode,
                pressed,
            } => {
                if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
                    && let Some(info) = self.surfaces.get(&obj_id)
                    && let Some(toplevel) = info.window.toplevel()
                    && let Some(keyboard) = self.seat.get_keyboard()
                {
                    if self.verbose {
                        eprintln!("[compositor] key: sid={surface_id} evdev={keycode} pressed={pressed}");
                    }
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = elapsed_ms();
                    let state = if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    };
                    keyboard.set_focus(
                        self,
                        Some(toplevel.wl_surface().clone()),
                        serial,
                    );
                    keyboard.input::<(), _>(
                        self,
                        keycode.into(),
                        state,
                        serial,
                        time,
                        |_, _, _| FilterResult::Forward,
                    );
                }
            }
            CompositorCommand::PointerMotion { surface_id, x, y } => {
                if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
                    && let Some(info) = self.surfaces.get(&obj_id)
                    && let Some(toplevel) = info.window.toplevel()
                    && let Some(pointer) = self.seat.get_pointer()
                {
                    let serial = SERIAL_COUNTER.next_serial();
                    let wl_surface = toplevel.wl_surface().clone();
                    pointer.motion(
                        self,
                        Some((wl_surface, (0.0, 0.0).into())),
                        &MotionEvent {
                            location: (x, y).into(),
                            serial,
                            time: elapsed_ms(),
                        },
                    );
                    pointer.frame(self);
                }
            }
            CompositorCommand::PointerButton {
                surface_id,
                button,
                pressed,
            } => {
                if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
                    && self.surfaces.contains_key(&obj_id)
                    && let Some(pointer) = self.seat.get_pointer()
                {
                    let serial = SERIAL_COUNTER.next_serial();
                    let state = if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    };
                    pointer.button(
                        self,
                        &ButtonEvent {
                            button,
                            state,
                            serial,
                            time: elapsed_ms(),
                        },
                    );
                    pointer.frame(self);
                }
            }
            CompositorCommand::PointerAxis {
                surface_id: _,
                axis,
                value,
            } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let ax = if axis == 0 {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                };
                pointer.axis(self, AxisFrame::new(elapsed_ms()).value(ax, value));
                pointer.frame(self);
            }
            CompositorCommand::SurfaceResize {
                surface_id: _,
                width,
                height,
                scale_120,
            } => {
                // Update output scale if the client reported a DPR.
                // scale_120 is already in Wayland fractional_scale units (1/120th).
                let scale_frac = if scale_120 >= 120 { scale_120 as f64 } else { 120.0 };
                let cur = self.output.current_scale().fractional_scale();
                if (cur - scale_frac).abs() > 0.01 {
                    // Integer scale for wl_output: round to nearest.
                    let int_scale = ((scale_frac / 120.0) + 0.5) as i32;
                    self.output.change_current_state(
                        None,
                        None,
                        Some(smithay::output::Scale::Custom {
                            advertised_integer: int_scale.max(1),
                            fractional: scale_frac,
                        }),
                        None,
                    );
                }

                // width/height are in physical pixels.  Convert to logical
                // pixels for the toplevel configure (Wayland uses logical).
                let scale_f = scale_frac / 120.0;
                let logical_w = ((width as f64) / scale_f).round() as i32;
                let logical_h = ((height as f64) / scale_f).round() as i32;

                // Update the output mode to match the physical size.
                let mode = smithay::output::Mode {
                    size: (width as i32, height as i32).into(),
                    refresh: 60_000,
                };
                self.output.change_current_state(Some(mode), None, None, None);
                self.output.set_preferred(mode);

                // Configure all toplevel surfaces to fill the output.
                for info in self.surfaces.values() {
                    if let Some(toplevel) = info.window.toplevel() {
                        toplevel.with_pending_state(|state| {
                            state.size =
                                Some((logical_w.max(1), logical_h.max(1)).into());
                        });
                        toplevel.send_pending_configure();
                    }
                }
            }
            CompositorCommand::SurfaceFocus { surface_id } => {
                if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
                    && let Some(info) = self.surfaces.get(&obj_id)
                    && let Some(toplevel) = info.window.toplevel()
                    && let Some(keyboard) = self.seat.get_keyboard()
                {
                    let serial = SERIAL_COUNTER.next_serial();
                    keyboard.set_focus(
                        self,
                        Some(toplevel.wl_surface().clone()),
                        serial,
                    );
                }
            }
            CompositorCommand::ClipboardOffer { .. } => {}
            CompositorCommand::Capture { surface_id, reply } => {
                let result = if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
                    && let Some(info) = self.surfaces.get(&obj_id)
                    && let Some(toplevel) = info.window.toplevel()
                {
                    let wl_surface = toplevel.wl_surface().clone();
                    self.read_surface_pixels(&wl_surface)
                } else {
                    None
                };
                let _ = reply.send(result);
            }
            CompositorCommand::RequestFrame { surface_id } => {
                self.fire_frame_callbacks(surface_id);
            }
            CompositorCommand::Shutdown => {
                self.loop_signal.stop();
            }
        }
    }

    /// Fire pending `wl_surface.frame` callbacks for a specific surface.
    fn fire_frame_callbacks(&self, surface_id: u16) {
        if let Some(&obj_id) = self.surface_lookup.get(&surface_id)
            && let Some(info) = self.surfaces.get(&obj_id)
            && let Some(toplevel) = info.window.toplevel()
        {
            let surface = toplevel.wl_surface().clone();
            let time = elapsed_ms();
            let mut fired = 0u32;
            with_surface_tree_downward(
                &surface,
                (),
                |_, _, &()| TraversalAction::DoChildren(()),
                |_, states, &()| {
                    for callback in states
                        .cached_state
                        .get::<SurfaceAttributes>()
                        .current()
                        .frame_callbacks
                        .drain(..)
                    {
                        callback.done(time);
                        fired += 1;
                    }
                },
                |_, _, &()| true,
            );
            if fired > 0 && self.verbose {
                eprintln!("[compositor] fire_frame_callbacks sid={surface_id}: {fired}");
            }
        }
    }

    fn read_surface_pixels(&mut self, surface: &WlSurface) -> Option<(u32, u32, Vec<u8>)> {
        let mut result = None;
        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attrs = guard.current();
            if let Some(compositor::BufferAssignment::NewBuffer(buffer)) = attrs.buffer.as_ref() {
                let shm_ok = with_buffer_contents(buffer, |ptr, len, data: BufferData| {
                    let width = data.width as u32;
                    let height = data.height as u32;
                    let stride = data.stride as usize;
                    let offset = data.offset as usize;
                    let pixel_data = unsafe { std::slice::from_raw_parts(ptr, len) };
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for row in 0..height as usize {
                        let row_start = offset + row * stride;
                        let row_end = row_start + (width as usize * 4);
                        if row_end <= pixel_data.len() {
                            for px in pixel_data[row_start..row_end].chunks_exact(4) {
                                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                            }
                        }
                    }
                    result = Some((width, height, rgba));
                })
                .is_ok();

                if !shm_ok && let Ok(dmabuf) = get_dmabuf(buffer) {
                    result = read_dmabuf_pixels(dmabuf);
                }
            }
        });
        result
    }
}

fn elapsed_ms() -> u32 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

impl CompositorHandler for Compositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientData>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        let key = surface.id().protocol_id() as u64;
        let surface_id = match self.surfaces.get(&key) {
            Some(info) => info.surface_id,
            None => return,
        };

        let mut committed_buffer = None;
        let mut new_title = String::new();
        let mut new_app_id = String::new();

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attrs = guard.current();
            if let Some(compositor::BufferAssignment::NewBuffer(buffer)) = attrs.buffer.as_ref() {
                let shm_ok = with_buffer_contents(buffer, |ptr, len, data: BufferData| {
                    let width = data.width as u32;
                    let height = data.height as u32;
                    let stride = data.stride as usize;
                    let offset = data.offset as usize;
                    let pixel_data = unsafe { std::slice::from_raw_parts(ptr, len) };
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for row in 0..height as usize {
                        let row_start = offset + row * stride;
                        let row_end = row_start + (width as usize * 4);
                        if row_end <= pixel_data.len() {
                            for px in pixel_data[row_start..row_end].chunks_exact(4) {
                                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                            }
                        }
                    }
                    committed_buffer = Some((width, height, rgba));
                })
                .is_ok();

                if !shm_ok && let Ok(dmabuf) = get_dmabuf(buffer) {
                    committed_buffer = read_dmabuf_pixels(dmabuf);
                }
                if committed_buffer.is_none() {
                    eprintln!(
                        "compositor: commit with no readable buffer (shm_ok={shm_ok}, has_dmabuf={})",
                        get_dmabuf(buffer).is_ok()
                    );
                }
                // We've copied the pixels — release the buffer so the
                // client can reuse it for the next frame.  Without this,
                // clients like Firefox block waiting for the release
                // event and never commit again.
                buffer.release();
            }

            if let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() {
                let lock = data.lock().unwrap();
                new_title = lock.title.clone().unwrap_or_default();
                new_app_id = lock.app_id.clone().unwrap_or_default();
            }
        });

        if let Some(info) = self.surfaces.get_mut(&key)
            && new_title != info.last_title
        {
            info.last_title = new_title.clone();
            let _ = self.event_tx.send(CompositorEvent::SurfaceTitle {
                surface_id,
                title: new_title,
            });
        }

        if let Some(info) = self.surfaces.get_mut(&key)
            && new_app_id != info.last_app_id
        {
            info.last_app_id = new_app_id.clone();
            let _ = self.event_tx.send(CompositorEvent::SurfaceAppId {
                surface_id,
                app_id: new_app_id,
            });
        }

        if let Some((width, height, pixels)) = committed_buffer {
            let info = self.surfaces.get_mut(&key).unwrap();
            if width != info.last_width || height != info.last_height {
                info.last_width = width;
                info.last_height = height;
                let _ = self.event_tx.send(CompositorEvent::SurfaceResized {
                    surface_id,
                    width: width as u16,
                    height: height as u16,
                });
            }

            if !pixels.is_empty() {
                let _ = self.event_tx.send(CompositorEvent::SurfaceCommit {
                    surface_id,
                    width,
                    height,
                    pixels,
                });
                (self.event_notify)();
            }
        }

        let time = elapsed_ms();
        with_surface_tree_downward(
            surface,
            (),
            |_, _, &()| TraversalAction::DoChildren(()),
            |_, states, &()| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            },
            |_, _, &()| true,
        );
    }
}

impl BufferHandler for Compositor {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Compositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        if self.verbose {
            eprintln!("[compositor] new_toplevel");
        }
        let window = Window::new_wayland_window(surface.clone());
        let wl_surface = surface.wl_surface().clone();
        let key = wl_surface.id().protocol_id() as u64;
        let surface_id = self.allocate_surface_id();

        self.space.map_element(window.clone(), (0, 0), false);

        let info = SurfaceInfo {
            surface_id,
            window,
            last_width: 0,
            last_height: 0,
            last_title: String::new(),
            last_app_id: String::new(),
        };
        self.surfaces.insert(key, info);
        self.surface_lookup.insert(surface_id, key);

        surface.with_pending_state(|state| {
            state.states.set(
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated,
            );
        });
        surface.send_configure();

        let _ = self.event_tx.send(CompositorEvent::SurfaceCreated {
            surface_id,
            title: String::new(),
            app_id: String::new(),
            parent_id: 0,
            width: 0,
            height: 0,
        });
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let key = wl_surface.id().protocol_id() as u64;
        if let Some(info) = self.surfaces.remove(&key) {
            self.surface_lookup.remove(&info.surface_id);
            self.space.unmap_elem(&info.window);
            let _ = self.event_tx.send(CompositorEvent::SurfaceDestroyed {
                surface_id: info.surface_id,
            });
        }
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl OutputHandler for Compositor {}

impl SeatHandler for Compositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
}

impl SelectionHandler for Compositor {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Compositor {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Compositor {}
impl ServerDndGrabHandler for Compositor {}

impl XdgDecorationHandler for Compositor {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }
}

impl DmabufHandler for Compositor {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let _ = notifier.successful::<Compositor>();
    }
}

fn with_dmabuf_plane_bytes<T>(
    dmabuf: &Dmabuf,
    plane_idx: usize,
    f: impl FnOnce(&[u8]) -> Option<T>,
) -> Option<T> {
    let _ = dmabuf.sync_plane(
        plane_idx,
        smithay::backend::allocator::dmabuf::DmabufSyncFlags::START
            | smithay::backend::allocator::dmabuf::DmabufSyncFlags::READ,
    );
    struct PlaneSyncGuard<'a> {
        dmabuf: &'a Dmabuf,
        plane_idx: usize,
    }

    impl Drop for PlaneSyncGuard<'_> {
        fn drop(&mut self) {
            let _ = self.dmabuf.sync_plane(
                self.plane_idx,
                smithay::backend::allocator::dmabuf::DmabufSyncFlags::END
                    | smithay::backend::allocator::dmabuf::DmabufSyncFlags::READ,
            );
        }
    }

    let _sync_guard = PlaneSyncGuard { dmabuf, plane_idx };
    let mapping = dmabuf.map_plane(plane_idx, DmabufMappingMode::READ).ok()?;
    let ptr = mapping.ptr() as *const u8;
    let len = mapping.length();
    let plane_data = unsafe { std::slice::from_raw_parts(ptr, len) };
    f(plane_data)
}

fn yuv420_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let y = (y as i32 - 16).max(0);
    let u = u as i32 - 128;
    let v = v as i32 - 128;

    let r = ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
    let g = ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
    let b = ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8;

    [r, g, b]
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let raw = bytes.get(offset..end)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_packed_rgba_dmabuf(
    plane_data: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    y_inverted: bool,
    format: Fourcc,
) -> Option<Vec<u8>> {
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let src_row = if y_inverted { height - 1 - row } else { row };
        let row_start = src_row * stride;
        let row_end = row_start + (width * 4);
        if row_end > plane_data.len() {
            return None;
        }

        for col in 0..width {
            let i = row_start + col * 4;
            match format {
                Fourcc::Argb8888 => {
                    rgba.push(plane_data[i + 2]);
                    rgba.push(plane_data[i + 1]);
                    rgba.push(plane_data[i]);
                    rgba.push(plane_data[i + 3]);
                }
                Fourcc::Xrgb8888 => {
                    rgba.push(plane_data[i + 2]);
                    rgba.push(plane_data[i + 1]);
                    rgba.push(plane_data[i]);
                    rgba.push(255);
                }
                Fourcc::Abgr8888 => {
                    rgba.push(plane_data[i]);
                    rgba.push(plane_data[i + 1]);
                    rgba.push(plane_data[i + 2]);
                    rgba.push(plane_data[i + 3]);
                }
                Fourcc::Xbgr8888 => {
                    rgba.push(plane_data[i]);
                    rgba.push(plane_data[i + 1]);
                    rgba.push(plane_data[i + 2]);
                    rgba.push(255);
                }
                _ => return None,
            }
        }
    }
    Some(rgba)
}

fn read_nv12_dmabuf(
    y_plane: &[u8],
    y_stride: usize,
    uv_plane: &[u8],
    uv_stride: usize,
    width: usize,
    height: usize,
    y_inverted: bool,
) -> Option<Vec<u8>> {
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return None;
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let src_row = if y_inverted { height - 1 - row } else { row };
        let y_row_start = src_row * y_stride;
        let y_row_end = y_row_start + width;
        if y_row_end > y_plane.len() {
            return None;
        }

        let uv_row_start = (src_row / 2) * uv_stride;
        let uv_row_end = uv_row_start + width;
        if uv_row_end > uv_plane.len() {
            return None;
        }

        for col in 0..width {
            let y = y_plane[y_row_start + col];
            let uv_idx = uv_row_start + (col / 2) * 2;
            let u = uv_plane[uv_idx];
            let v = uv_plane[uv_idx + 1];
            let [r, g, b] = yuv420_to_rgb(y, u, v);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    Some(rgba)
}

fn read_p010_dmabuf(
    y_plane: &[u8],
    y_stride: usize,
    uv_plane: &[u8],
    uv_stride: usize,
    width: usize,
    height: usize,
    y_inverted: bool,
) -> Option<Vec<u8>> {
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return None;
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let src_row = if y_inverted { height - 1 - row } else { row };
        let y_row_start = src_row * y_stride;
        let y_row_end = y_row_start + width * 2;
        if y_row_end > y_plane.len() {
            return None;
        }

        let uv_row_start = (src_row / 2) * uv_stride;
        let uv_row_end = uv_row_start + width * 2;
        if uv_row_end > uv_plane.len() {
            return None;
        }

        for col in 0..width {
            let y = (read_le_u16(y_plane, y_row_start + col * 2)? >> 8) as u8;
            let uv_idx = uv_row_start + (col / 2) * 4;
            let u = (read_le_u16(uv_plane, uv_idx)? >> 8) as u8;
            let v = (read_le_u16(uv_plane, uv_idx + 2)? >> 8) as u8;
            let [r, g, b] = yuv420_to_rgb(y, u, v);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    Some(rgba)
}

fn read_dmabuf_pixels(dmabuf: &Dmabuf) -> Option<(u32, u32, Vec<u8>)> {
    let size = dmabuf.size();
    let width = size.w as u32;
    let height = size.h as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let format = dmabuf.format();
    // We attempt to mmap regardless of modifier.  For software renderers
    // (llvmpipe, swrast) the buffer is linear in memory even when the
    // advertised modifier says otherwise.  If the kernel can't provide a
    // readable mapping for a truly tiled buffer, map_plane() will fail
    // and we'll return None naturally.
    eprintln!(
        "read_dmabuf_pixels: {width}x{height} fourcc={:?} modifier={:?}",
        format.code, format.modifier
    );

    let width_usize = width as usize;
    let height_usize = height as usize;
    let y_inverted = dmabuf.y_inverted();
    let rgba = match format.code {
        Fourcc::Argb8888 | Fourcc::Xrgb8888 | Fourcc::Abgr8888 | Fourcc::Xbgr8888 => {
            let stride = dmabuf.strides().next().unwrap_or(width * 4) as usize;
            with_dmabuf_plane_bytes(dmabuf, 0, |plane_data| {
                read_packed_rgba_dmabuf(
                    plane_data,
                    stride,
                    width_usize,
                    height_usize,
                    y_inverted,
                    format.code,
                )
            })?
        }
        Fourcc::Nv12 => {
            let mut strides = dmabuf.strides();
            let y_stride = strides.next().unwrap_or(width) as usize;
            let uv_stride = strides.next().unwrap_or(width) as usize;
            with_dmabuf_plane_bytes(dmabuf, 0, |y_plane| {
                with_dmabuf_plane_bytes(dmabuf, 1, |uv_plane| {
                    read_nv12_dmabuf(
                        y_plane,
                        y_stride,
                        uv_plane,
                        uv_stride,
                        width_usize,
                        height_usize,
                        y_inverted,
                    )
                })
            })?
        }
        Fourcc::P010 => {
            let mut strides = dmabuf.strides();
            let y_stride = strides.next().unwrap_or(width * 2) as usize;
            let uv_stride = strides.next().unwrap_or(width * 2) as usize;
            with_dmabuf_plane_bytes(dmabuf, 0, |y_plane| {
                with_dmabuf_plane_bytes(dmabuf, 1, |uv_plane| {
                    read_p010_dmabuf(
                        y_plane,
                        y_stride,
                        uv_plane,
                        uv_stride,
                        width_usize,
                        height_usize,
                        y_inverted,
                    )
                })
            })?
        }
        _ => return None,
    };

    Some((width, height, rgba))
}

impl PrimarySelectionHandler for Compositor {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl XdgActivationHandler for Compositor {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        _token_data: XdgActivationTokenData,
        _surface: WlSurface,
    ) {
    }
}

impl FractionalScaleHandler for Compositor {
    fn new_fractional_scale(&mut self, _surface: WlSurface) {}
}

impl XdgToplevelIconHandler for Compositor {}
impl TabletSeatHandler for Compositor {}

delegate_compositor!(Compositor);
delegate_cursor_shape!(Compositor);
delegate_shm!(Compositor);
delegate_xdg_shell!(Compositor);
delegate_seat!(Compositor);
delegate_data_device!(Compositor);
delegate_primary_selection!(Compositor);
delegate_output!(Compositor);
delegate_dmabuf!(Compositor);
delegate_fractional_scale!(Compositor);
delegate_viewporter!(Compositor);
delegate_xdg_activation!(Compositor);
delegate_xdg_decoration!(Compositor);
delegate_xdg_toplevel_icon!(Compositor);
delegate_text_input_manager!(Compositor);

pub struct CompositorHandle {
    pub event_rx: mpsc::Receiver<CompositorEvent>,
    pub command_tx: mpsc::Sender<CompositorCommand>,
    pub socket_name: String,
    pub thread: std::thread::JoinHandle<()>,
    pub shutdown: Arc<AtomicBool>,
    loop_signal: LoopSignal,
}

impl CompositorHandle {
    /// Wake the compositor event loop immediately so it processes
    /// pending commands without waiting for the idle timeout.
    pub fn wake(&self) {
        self.loop_signal.wakeup();
    }
}

pub fn spawn_compositor(
    verbose: bool,
    event_notify: Arc<dyn Fn() + Send + Sync>,
) -> CompositorHandle {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let (socket_tx, socket_rx) = mpsc::sync_channel(1);
    let (signal_tx, signal_rx) = mpsc::sync_channel::<LoopSignal>(1);
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| {
            // Verify the directory is actually writable by the current user
            // before using it. A stale or inaccessible XDG_RUNTIME_DIR (e.g.
            // in containers, after su/sudo, or in CI) causes PermissionDenied
            // when smithay tries to bind the Wayland socket. A probe write is
            // more reliable than inspecting mode bits, which don't account for
            // the effective uid.
            let probe = p.join(".blit-probe");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                true
            } else {
                false
            }
        })
        .unwrap_or_else(std::env::temp_dir);

    let runtime_dir_clone = runtime_dir.clone();
    let thread = std::thread::spawn(move || {
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir_clone) };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_compositor(event_tx, command_rx, socket_tx, signal_tx, event_notify, shutdown_clone, verbose);
        }));
        if let Err(e) = result {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("[compositor] PANIC: {msg}");
        }
    });

    let socket_name = socket_rx.recv().expect("compositor failed to start");
    let socket_name = runtime_dir
        .join(&socket_name)
        .to_string_lossy()
        .into_owned();
    let loop_signal = signal_rx.recv().expect("compositor failed to send loop signal");

    CompositorHandle {
        event_rx,
        command_tx,
        socket_name,
        thread,
        shutdown,
        loop_signal,
    }
}

fn run_compositor(
    event_tx: mpsc::Sender<CompositorEvent>,
    command_rx: mpsc::Receiver<CompositorCommand>,
    socket_tx: mpsc::SyncSender<String>,
    signal_tx: mpsc::SyncSender<LoopSignal>,
    event_notify: Arc<dyn Fn() + Send + Sync>,
    shutdown: Arc<AtomicBool>,
    verbose: bool,
) {
    let mut event_loop: EventLoop<Compositor> =
        EventLoop::try_new().expect("failed to create event loop");
    let display: Display<Compositor> = Display::new().expect("failed to create display");
    let dh = display.handle();

    let compositor_state = CompositorState::new::<Compositor>(&dh);
    let xdg_shell_state = XdgShellState::new::<Compositor>(&dh);
    let shm_state = ShmState::new::<Compositor>(&dh, vec![]);
    let data_device_state = DataDeviceState::new::<Compositor>(&dh);
    let viewporter_state = ViewporterState::new::<Compositor>(&dh);
    let xdg_decoration_state = XdgDecorationState::new::<Compositor>(&dh);
    let primary_selection_state = PrimarySelectionState::new::<Compositor>(&dh);
    let activation_state = XdgActivationState::new::<Compositor>(&dh);
    FractionalScaleManagerState::new::<Compositor>(&dh);
    CursorShapeManagerState::new::<Compositor>(&dh);
    // Disabled: smithay 0.7 has a bug in ShmBufferUserData::remove_destruction_hook
    // (uses != instead of ==) that causes a protocol error when clients destroy icon
    // buffers, killing Chromium-based browsers.
    TextInputManagerState::new::<Compositor>(&dh);

    let mut dmabuf_state = DmabufState::new();
    let dmabuf_formats = [
        DmabufFormat {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::Abgr8888,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::Xbgr8888,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::Nv12,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::P010,
            modifier: Modifier::Linear,
        },
        DmabufFormat {
            code: Fourcc::Argb8888,
            modifier: Modifier::Invalid,
        },
        DmabufFormat {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Invalid,
        },
        DmabufFormat {
            code: Fourcc::Abgr8888,
            modifier: Modifier::Invalid,
        },
        DmabufFormat {
            code: Fourcc::Xbgr8888,
            modifier: Modifier::Invalid,
        },
        DmabufFormat {
            code: Fourcc::Nv12,
            modifier: Modifier::Invalid,
        },
        DmabufFormat {
            code: Fourcc::P010,
            modifier: Modifier::Invalid,
        },
    ];
    let dmabuf_global = dmabuf_state.create_global::<Compositor>(&dh, dmabuf_formats);

    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(&dh, "headless");
    seat.add_keyboard(XkbConfig::default(), 200, 25)
        .expect("failed to add keyboard");
    seat.add_pointer();

    let output = Output::new(
        "headless-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Virtual".into(),
            model: "Headless".into(),
        },
    );
    let mode = Mode {
        size: (1920, 1080).into(),
        refresh: 60_000,
    };
    output.create_global::<Compositor>(&dh);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    let mut space = Space::default();
    space.map_output(&output, (0, 0));

    let renderer = PixmanRenderer::new().expect("failed to create pixman renderer");

    let listening_socket = ListeningSocketSource::new_auto().unwrap_or_else(|e| {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "(unset)".into());
        panic!(
            "failed to create wayland socket in XDG_RUNTIME_DIR={dir}: {e}\n\
             hint: ensure the directory exists and is writable by the current user"
        );
    });
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    socket_tx.send(socket_name).unwrap();

    let handle = event_loop.handle();

    handle
        .insert_source(listening_socket, |client_stream, _, state| {
            if let Err(e) = state.display_handle.insert_client(
                client_stream,
                Arc::new(ClientData {
                    compositor_state: CompositorClientState::default(),
                }),
            ) && verbose
            {
                eprintln!("[compositor] insert_client error: {e}");
            }
        })
        .expect("failed to insert listening socket");

    let loop_signal = event_loop.get_signal();

    let mut compositor = Compositor {
        display_handle: dh.clone(),
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
        data_device_state,
        viewporter_state,
        xdg_decoration_state,
        dmabuf_state,
        dmabuf_global,
        primary_selection_state,
        activation_state,
        seat,
        output,
        space,
        surfaces: HashMap::new(),
        surface_lookup: HashMap::new(),
        next_surface_id: 1,
        event_tx,
        event_notify,
        loop_signal: loop_signal.clone(),
        renderer,
        verbose,
    };

    // Send the loop signal back so the server can wake us.
    let _ = signal_tx.send(loop_signal.clone());

    let display_source = Generic::new(display, Interest::READ, calloop::Mode::Level);
    handle
        .insert_source(display_source, |_, display, state| {
            let d = unsafe { display.get_mut() };
            if let Err(e) = d.dispatch_clients(state)
                && verbose
            {
                eprintln!("[compositor] dispatch_clients error: {e}");
            }
            if let Err(e) = d.flush_clients()
                && verbose
            {
                eprintln!("[compositor] flush_clients error: {e}");
            }
            Ok(PostAction::Continue)
        })
        .expect("failed to insert display source");

    if verbose {
        eprintln!("[compositor] entering event loop");
    }
    while !shutdown.load(Ordering::Relaxed) {
        while let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                CompositorCommand::Shutdown => {
                    shutdown.store(true, Ordering::Relaxed);
                    return;
                }
                other => compositor.handle_command(other),
            }
        }

        // No rate limit — the loop wakes instantly on Wayland client
        // traffic (fd readable) or server commands (loop_signal.wakeup()).
        // The 1s ceiling is only a liveness fallback for shutdown polling.
        if let Err(e) =
            event_loop.dispatch(Some(std::time::Duration::from_secs(1)), &mut compositor)
            && verbose
        {
            eprintln!("[compositor] event loop error: {e}");
        }

        if let Err(e) = compositor.display_handle.flush_clients()
            && verbose
        {
            eprintln!("[compositor] flush error: {e}");
        }
    }
    if verbose {
        eprintln!("[compositor] event loop exited");
    }
}

#[cfg(test)]
mod tests {
    use super::{Fourcc, read_nv12_dmabuf, read_p010_dmabuf, read_packed_rgba_dmabuf};

    #[test]
    fn xrgb_dmabuf_forces_opaque_alpha() {
        let pixels = [
            0x10, 0x20, 0x30, 0x00, //
            0x40, 0x50, 0x60, 0x7f,
        ];

        let rgba = read_packed_rgba_dmabuf(&pixels, 8, 2, 1, false, Fourcc::Xrgb8888).unwrap();

        assert_eq!(rgba, vec![0x30, 0x20, 0x10, 0xff, 0x60, 0x50, 0x40, 0xff]);
    }

    #[test]
    fn nv12_black_decodes_to_opaque_black() {
        let y_plane = [16, 16, 16, 16];
        let uv_plane = [128, 128];

        let rgba = read_nv12_dmabuf(&y_plane, 2, &uv_plane, 2, 2, 2, false).unwrap();

        assert_eq!(rgba, vec![0, 0, 0, 255].repeat(4));
    }

    #[test]
    fn p010_white_decodes_to_opaque_white() {
        let y_plane = [0x00, 0xeb, 0x00, 0xeb, 0x00, 0xeb, 0x00, 0xeb];
        let uv_plane = [0x00, 0x80, 0x00, 0x80];

        let rgba = read_p010_dmabuf(&y_plane, 4, &uv_plane, 4, 2, 2, false).unwrap();

        assert_eq!(rgba, vec![255, 255, 255, 255].repeat(4));
    }
}
