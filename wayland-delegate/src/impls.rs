use proc_macro2::{TokenStream, Span};
use quote::{quote_spanned, format_ident};
use syn::{parse2 as parse, ItemImpl, Generics, Type};

macro_rules! quote {
    ($($tt:tt)*) => { quote_spanned! { Span::mixed_site() => $($tt)* } };
}

/// Composes the delegation implementations
fn delegator (input: TokenStream, globals: &[TokenStream], locals: &[TokenStream]) -> TokenStream {
    quote! {
        #(#globals)*
        #(#locals)*
        #input
    }
}

pub fn delegate (
    generics: &Generics,
    source: &Box<Type>,
    target: &TokenStream,
    interface: TokenStream,
    data: TokenStream,
) -> TokenStream {
    quote! {
        impl #generics wayland_server::Dispatch<#interface, #data> for #source {
            fn request (
                state:     &mut Self,
                client:    &wayland_server::Client,
                resource:  &#interface,
                request:   <#interface as wayland_server::Resource>::Request,
                data:      &#data,
                dhandle:   &wayland_server::DisplayHandle,
                data_init: &mut wayland_server::DataInit<'_, Self>,
            ) {
                <#target as wayland_server::Dispatch<#interface, #data, Self>>::request(
                    state, client, resource, request, data, dhandle, data_init
                )
            }
            fn destroyed (
                state:    &mut Self,
                client:   wayland_server::backend::ClientId,
                resource: wayland_server::backend::ObjectId,
                data:     &#data
            ) {
                <#target as wayland_server::Dispatch<#interface, #data, Self>>::destroyed(
                    state, client, resource, data
                )
            }
        }
    }
}

pub fn delegate_global (
    generics: &Generics,
    source: &Box<Type>,
    target: &TokenStream,
    interface: TokenStream,
    data: TokenStream,
) -> TokenStream {
    quote! {
        impl #generics wayland_server::GlobalDispatch<#interface, #data> for #source {
            fn bind (
                state:       &mut Self,
                dhandle:     &wayland_server::DisplayHandle,
                client:      &wayland_server::Client,
                resource:    wayland_server::New<#interface>,
                global_data: &#data,
                data_init:   &mut wayland_server::DataInit<'_, Self>,
            ) {
                <#target as wayland_server::GlobalDispatch<#interface, #data, Self>>::bind(
                    state, dhandle, client, resource, global_data, data_init
                )
            }
            fn can_view (
                client:      wayland_server::Client,
                global_data: &#data
            ) -> bool {
                <#target as wayland_server::GlobalDispatch<#interface, #data, Self>>::can_view(
                    client, global_data
                )
            }
        }
    }
}

pub fn delegate_output (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { OutputManagerState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_output::WlOutput
        }, quote! {
            smithay::wayland::output::WlOutputData
        }),
        delegate_global(&g, &s, &t, quote! {
            wayland_protocols::xdg::xdg_output::zv1::server::zxdg_output_manager_v1::ZxdgOutputManagerV1
        }, quote! { () })
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_output::WlOutput
        }, quote! {
            smithay::wayland::output::OutputUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::xdg_output::zv1::server::zxdg_output_v1::ZxdgOutputV1
        }, quote! {
            smithay::wayland::output::XdgOutputUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::xdg_output::zv1::server::zxdg_output_manager_v1::ZxdgOutputManagerV1
        }, quote! {
            ()
        })
    ])
}

pub fn delegate_compositor (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { CompositorState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_compositor::WlCompositor
        }, quote! {
            ()
        }),
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_subcompositor::WlSubcompositor
        }, quote! {
            ()
        }),
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_compositor::WlCompositor
        }, quote! {
            ()
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_surface::WlSurface
        }, quote! {
            smithay::wayland::compositor::SurfaceUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_region::WlRegion
        }, quote! {
            smithay::wayland::compositor::RegionUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_callback::WlCallback
        }, quote! {
            ()
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_subcompositor::WlSubcompositor
        }, quote! {
            ()
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_subsurface::WlSubsurface
        }, quote! {
            smithay::wayland::compositor::SubsurfaceUserData
        }),
    ])
}

pub fn delegate_xdg_shell (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { XdgShellState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase
        }, quote! {
            ()
        }),
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase
        }, quote! {
            smithay::wayland::shell::xdg::XdgWmBaseUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_positioner::XdgPositioner
        }, quote! {
            smithay::wayland::shell::xdg::XdgPositionerUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_popup::XdgPopup
        }, quote! {
            smithay::wayland::shell::xdg::XdgShellSurfaceUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_surface::XdgSurface
        }, quote! {
            smithay::wayland::shell::xdg::XdgSurfaceUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel
        }, quote! {
            smithay::wayland::shell::xdg::XdgShellSurfaceUserData
        }),
    ])
}

pub fn delegate_shm (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { ShmState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_shm::WlShm
        }, quote! {
            ()
        }),
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_shm::WlShm
        }, quote! {
            ()
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_shm_pool::WlShmPool
        }, quote! {
            smithay::wayland::shm::ShmPoolUserData
        }),
        delegate(&g, &s, &t, quote! {
            WlBuffer
        }, quote! {
            smithay::wayland::shm::ShmBufferUserData
        }),
    ])
}

pub fn delegate_dmabuf (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { DmabufState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1
        }, quote! {
            smithay::wayland::dmabuf::DmabufGlobalData
        }),
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1
        }, quote! {
            smithay::wayland::dmabuf::DmabufData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1
        }, quote! {
            smithay::wayland::dmabuf::DmabufParamsData
        }),
        delegate(&g, &s, &t, quote! { WlBuffer }, quote! { Dmabuf }),
    ])
}

pub fn delegate_fractional_scale (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { FractionalScaleManagerState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! { WpFractionalScaleManagerV1 }, quote! { Logger }),
    ], &[
        delegate(&g, &s, &t, quote! { WpFractionalScaleManagerV1 }, quote! { Logger }),
        delegate(&g, &s, &t, quote! { WpFractionalScaleV1 }, quote! { WlSurface }),
    ])
}

pub fn delegate_presentation (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { PresentationState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! { WpPresentation }, quote! { u32 }),
    ], &[
        delegate(&g, &s, &t, quote! { WpPresentation }, quote! { u32 }),
        delegate(&g, &s, &quote! { PresentationFeedbackState },
            quote! { WpPresentationFeedback },
            quote! { () }, ),
    ])
}

pub fn delegate_seat (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { SeatState<#s> };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_seat::WlSeat
        }, quote! {
            smithay::wayland::seat::SeatGlobalData<#s>
        }),
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_seat::WlSeat
        }, quote! {
            smithay::wayland::seat::SeatUserData<#s>
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_pointer::WlPointer
        }, quote! {
            smithay::wayland::seat::PointerUserData<#s>
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_keyboard::WlKeyboard
        }, quote! {
            smithay::wayland::seat::KeyboardUserData<#s>
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_touch::WlTouch
        }, quote! {
            smithay::wayland::seat::TouchUserData
        }),
    ])
}

pub fn delegate_data_device (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input.clone()).unwrap();
    let t = quote! { DataDeviceState };
    delegator(input, &[
        delegate_global(&g, &s, &t, quote! {
            wayland_server::protocol::wl_data_device_manager::WlDataDeviceManager
        }, quote! {
            ()
        })
    ], &[
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_data_device_manager::WlDataDeviceManager
        }, quote! {
            ()
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_data_device::WlDataDevice
        }, quote! {
            smithay::wayland::data_device::DataDeviceUserData
        }),
        delegate(&g, &s, &t, quote! {
            wayland_server::protocol::wl_data_source::WlDataSource
        }, quote! {
            smithay::wayland::data_device::DataSourceUserData
        })
    ])
}

pub fn delegate_keyboard_shortcuts_inhibit (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_layer_shell (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_viewporter (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_primary_selection (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_input_method_manager (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_tablet_manager (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_text_input_manager (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_virtual_keyboard_manager (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_xdg_activation (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_xdg_decoration (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

pub fn delegate_kde_decoration (input: TokenStream) -> TokenStream {
    delegator(input, &[
    ], &[
    ])
}

