pub(crate) use crate::prelude::*;

pub(crate) use smithay::{
    delegate_compositor,
    delegate_data_device,
    delegate_output,
    delegate_seat,
    delegate_shm,
    delegate_xdg_shell,
    backend::renderer::utils::on_commit_buffer_handler,
    input::{
        SeatHandler,
        SeatState,
        pointer::{
            AxisFrame,
            ButtonEvent,
            Focus,
            GrabStartData as PointerGrabStartData,
            MotionEvent,
            PointerGrab,
            PointerInnerHandle,
        },
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::{
            State      as XdgToplevelState,
            ResizeEdge as XdgToplevelResizeEdge
        },
        wayland_server::{
            Client,
            DisplayHandle,
            Resource,
            protocol::{
                wl_seat::WlSeat,
                wl_buffer,
                wl_surface::WlSurface
            }
        },
        x11rb::{
            atom_manager,
            connection::Connection as _,
            errors::ReplyOrIdError,
            protocol::{
                composite::{
                    ConnectionExt as _,
                    Redirect
                },
                xproto::{
                    ChangeWindowAttributesAux,
                    ConfigWindow,
                    ConfigureWindowAux,
                    ConnectionExt as _,
                    EventMask,
                    Window as X11Window,
                    WindowClass,
                },
                Event,
            },
            rust_connection::{DefaultStream, RustConnection},
        }
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            self,
            CompositorHandler,
            CompositorState,
            get_parent,
            give_role,
            is_sync_subsurface,
            with_states,
        },
        data_device::{
            DataDeviceState,
            ClientDndGrabHandler,
            DataDeviceHandler,
            ServerDndGrabHandler
        },
        output::OutputManagerState,
        shm::{
            ShmHandler,
            ShmState
        },
        shell::xdg::{
            PopupSurface,
            PositionerState,
            ToplevelSurface,
            XdgShellHandler,
            XdgShellState,
            XdgToplevelSurfaceData,
            SurfaceCachedState
        },
    },
    xwayland::{
        XWayland,
        XWaylandEvent
    },
    desktop::{
        Kind,
        Space,
        Window,
        X11Surface
    },
    utils::{
        IsAlive,
        Serial,
        Transform,
        x11rb::X11Source,
    },
};

