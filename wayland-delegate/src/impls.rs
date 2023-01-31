use proc_macro2::TokenStream;
use quote::{quote, format_ident};
use syn::{parse2 as parse, ItemImpl, Generics, Type};

/// Composes the delegation implementations
fn delegator (globals: &[TokenStream], locals: &[TokenStream]) -> TokenStream {
    quote! {
        #(#globals)*
        #(#locals)*
    }
}

pub fn delegate (
    generics: &Generics,
    source: &Box<Type>,
    target: &TokenStream,
    interface: TokenStream,
    udata: TokenStream,
) -> TokenStream {
    quote! {
        impl #generics wayland_server::Dispatch<#interface, #udata> for #source {
            fn request (
                state:     &mut Self,
                client:    &wayland_server::Client,
                resource:  &$interface,
                request:   <$interface as wayland_server::Resource>::Request,
                data:      &$udata,
                dhandle:   &wayland_server::DisplayHandle,
                data_init: &mut wayland_server::DataInit<'_, Self>,
            ) {
                <$target as wayland_server::Dispatch<$interface, $udata, Self>>::request(
                    state, client, resource, request, data, dhandle, data_init
                )
            }
            fn destroyed (
                state:    &mut Self,
                client:   wayland_server::backend::ClientId,
                resource: wayland_server::backend::ObjectId,
                data:     &$udata
            ) {
                <$target as wayland_server::Dispatch<$interface, $udata, Self>>::destroyed(
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
    udata: TokenStream,
) -> TokenStream {
    quote! {
        impl #generics wayland_server::GlobalDispatch<#interface, #udata> for #source {
            fn bind (
                state:       &mut Self,
                dhandle:     &wayland_server::DisplayHandle,
                client:      &wayland_server::Client,
                resource:    wayland_server::New<$interface>,
                global_data: &$udata,
                data_init:   &mut wayland_server::DataInit<'_, Self>,
            ) {
                <$target as wayland_server::GlobalDispatch<$interface, $udata, Self>>::bind(
                    state, dhandle, client, resource, global_data, data_init
                )
            }
            fn can_view (
                client:      wayland_server::Client,
                global_data: &$udata
            ) -> bool {
                <$target as wayland_server::GlobalDispatch<$interface, $udata, Self>>::can_view(
                    client, global_data
                )
            }
        }
    }
}

pub fn delegate_output (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { OutputManagerState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WlOutput }, quote! { WlOutputData }),
        delegate_global(&g, &s, &t, quote! { ZxdgOutputManagerV1 }, quote! { () })
    ], &[
        delegate(&g, &s, &t, quote! { WlOutput }, quote! { OutputUserData }),
        delegate(&g, &s, &t, quote! { ZxdgOutputV1 }, quote! { XdgOutputUserData }),
        delegate(&g, &s, &t, quote! { ZxdgOutputManagerV1 }, quote! { () })
    ])
}

pub fn delegate_compositor (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { CompositorState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WlCompositor }, quote! { () }),
        delegate_global(&g, &s, &t, quote! { WlSubcompositor }, quote! { () }),
    ], &[
        delegate(&g, &s, &t, quote! { WlCompositor }, quote! { () }),
        delegate(&g, &s, &t, quote! { WlSurface }, quote! { SurfaceUserData }),
        delegate(&g, &s, &t, quote! { WlRegion }, quote! { RegionUserData }),
        delegate(&g, &s, &t, quote! { WlCallback }, quote! { () }),
        delegate(&g, &s, &t, quote! { WlSubcompositor }, quote! { () }),
        delegate(&g, &s, &t, quote! { WlSubsurface }, quote! { SubsurfaceUserData }),
    ])
}

pub fn delegate_xdg_shell (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { XdgShellState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { XdgWmBase }, quote! { () }),
    ], &[
        delegate(&g, &s, &t, quote! { XdgWmBase }, quote! { XdgWmBaseUserData }),
        delegate(&g, &s, &t, quote! { XdgPositioner }, quote! { XdgPositionerUserData }),
        delegate(&g, &s, &t, quote! { XdgPopup }, quote! { XdgShellSurfaceUserData }),
        delegate(&g, &s, &t, quote! { XdgSurface }, quote! { XdgSurfaceUserData }),
        delegate(&g, &s, &t, quote! { XdgToplevel }, quote! { XdgShellSurfaceUserData }),
    ])
}

pub fn delegate_shm (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { ShmState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WlShm }, quote! { () }),
    ], &[
        delegate(&g, &s, &t, quote! { WlShm }, quote! { () }),
        delegate(&g, &s, &t, quote! { WlShmPool }, quote! { ShmPoolUserData }),
        delegate(&g, &s, &t, quote! { WlBuffer }, quote! { ShmBuferUserData }),
    ])
}

pub fn delegate_dmabuf (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { DmabufState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { ZwpLinuxDmabufV1 }, quote! { DmabufGlobalData }),
    ], &[
        delegate(&g, &s, &t, quote! { ZwpLinuxDmabufV1 }, quote! { DmabufData }),
        delegate(&g, &s, &t, quote! { ZwpLinuxBufferParamsV1 }, quote! { DmabufParamsData }),
        delegate(&g, &s, &t, quote! { WlBuffer }, quote! { Dmabuf }),
    ])
}

pub fn delegate_fractional_scale (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { FractionalScaleManagerState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WpFractionalScaleManagerV1 }, quote! { Logger }),
    ], &[
        delegate(&g, &s, &t, quote! { WpFractionalScaleManagerV1 }, quote! { Logger }),
        delegate(&g, &s, &t, quote! { WpFractionalScaleV1 }, quote! { WlSurface }),
    ])
}

pub fn delegate_presentation (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { PresentationState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WpPresentation }, quote! { u32 }),
    ], &[
        delegate(&g, &s, &t, quote! { WpPresentation }, quote! { u32 }),
        delegate(&g, &s, &quote! { PresentationFeedbackState },
            quote! { WpPresentationFeedback },
            quote! { () }, ),
    ])
}

pub fn delegate_seat (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { SeatState<#s> };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WlSeat }, quote! { SeatGlobalData<#s> }),
    ], &[
        delegate(&g, &s, &t, quote! { WlSeat }, quote! { SeatUserData<#s> }),
        delegate(&g, &s, &t, quote! { WlPointer }, quote! { PointerUserData<#s> }),
        delegate(&g, &s, &t, quote! { WlKeyboard }, quote! { KeyboardUserData<#s> }),
        delegate(&g, &s, &t, quote! { WlTouch }, quote! { TouchUserData }),
    ])
}

pub fn delegate_data_device (input: TokenStream) -> TokenStream {
    let ItemImpl { generics: g, self_ty: s, .. } = parse(input).unwrap();
    let t = quote! { DataDeviceState };
    delegator(&[
        delegate_global(&g, &s, &t, quote! { WlDataDeviceManager }, quote! { () })
    ], &[
        delegate(&g, &s, &t, quote! { WlDataDeviceManager }, quote! { () }),
        delegate(&g, &s, &t, quote! { WlDataDevice }, quote! { DataDeviceUserData }),
        delegate(&g, &s, &t, quote! { WlDataSource }, quote! { DataSourceUserData })
    ])
}

pub fn delegate_keyboard_shortcuts_inhibit (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_layer_shell (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_viewporter (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_primary_selection (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_input_method_manager (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_tablet_manager (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_text_input_manager (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_virtual_keyboard_manager (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_xdg_activation (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_xdg_decoration (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

pub fn delegate_kde_decoration (input: TokenStream) -> TokenStream {
    delegator(&[
    ], &[
    ])
}

