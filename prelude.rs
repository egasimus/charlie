pub const OUTPUT_NAME: &str = "winit";

pub const BACKGROUND: &str = "data/cork.png";

pub(crate) use std::{
    cell::RefCell,
    rc::Rc,
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    time::Duration,
    error::Error,
};

pub(crate) use slog::{Logger, Drain, o, warn, error, info, debug};

pub(crate) use slog_scope::GlobalLoggerGuard;

pub(crate) use image::{self, ImageBuffer, Rgba};

pub(crate) use smithay::{
    backend::{
        SwapBuffersError,
        input::{
            Axis,
            AxisSource,
            ButtonState,
            Event,
            InputBackend,
            InputEvent,
            KeyState,
            KeyboardKeyEvent,
            MouseButton,
            PointerAxisEvent,
            PointerButtonEvent,
            PointerMotionAbsoluteEvent,
        },
        renderer::{
            BufferType,
            Frame,
            ImportAll,
            ImportDma,
            ImportEgl,
            Renderer,
            Texture,
            Transform,
            buffer_dimensions,
            buffer_type,
            gles2::{
                Gles2Renderer,
                Gles2Texture,
                Gles2Error
            }
        },
        winit::{
            self,
            WinitGraphicsBackend,
            WinitInputBackend
        }
    },
    reexports::{
        calloop::{
            EventLoop,
            Interest,
            Mode as CalloopMode,
            PostAction,
            LoopHandle,
            generic::{
                Generic
            },
        },
        wayland_protocols::xdg_shell::server::{
            xdg_toplevel::{
                self,
                ResizeEdge
            },
        },
        wayland_server::{
            Display,
            Global,
            UserDataMap,
            protocol::{
                wl_buffer,
                wl_output::{
                    self,
                    WlOutput
                },
                wl_pointer::{
                    self,
                    ButtonState as WlButtonState
                },
                wl_shell_surface::{
                    self,
                    Resize
                },
                wl_seat::{
                    WlSeat,
                },
                wl_surface::{
                    self,
                    WlSurface
                }
            },
        },
    },
    wayland::{
        SERIAL_COUNTER as SCOUNTER,
        dmabuf::init_dmabuf_global,
        compositor::{
            BufferAssignment,
            Damage,
            SubsurfaceCachedState,
            SurfaceAttributes,
            TraversalAction,
            compositor_init,
            get_role,
            is_sync_subsurface,
            with_states,
            with_surface_tree_downward,
            with_surface_tree_upward,
        },
        output::{
            self,
            Mode as OutputMode,
            PhysicalProperties,
            xdg::init_xdg_output_manager
        },
        seat::{
            AxisFrame,
            CursorImageAttributes,
            CursorImageStatus,
            GrabStartData,
            KeyboardHandle,
            Keysym,
            ModifiersState,
            PointerGrab,
            PointerHandle,
            PointerInnerHandle,
            Seat,
            XkbConfig,
            keysyms as xkb
        },
        shell::{
            legacy::{
                wl_shell_init,
                ShellRequest,
                ShellState as WlShellState,
                ShellSurface,
                ShellSurfaceKind
            },
            xdg::{
                Configure,
                PopupSurface,
                ShellState as XdgShellState,
                SurfaceCachedState,
                ToplevelConfigure,
                ToplevelSurface,
                XdgPopupSurfaceRoleAttributes,
                XdgRequest,
                XdgToplevelSurfaceRoleAttributes,
                xdg_shell_init,
            },
        },
        data_device::{
            default_action_chooser,
            init_data_device,
            set_data_device_focus,
            DataDeviceEvent
        },
        shm::init_shm_global,
        tablet_manager::{
            init_tablet_manager_global,
            TabletSeatTrait
        },
        Serial,
    },
    utils::{Logical, Physical, Point, Rectangle, Size},
};

