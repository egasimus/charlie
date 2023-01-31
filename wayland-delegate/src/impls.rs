use proc_macro2::TokenStream;
use quote::quote;

/// Composes the delegation implementations
fn delegator (globals: &[TokenStream], locals: &[TokenStream]) -> TokenStream {
    quote! {
        #(#globals)*
        #(#locals)*
    }
}

pub fn delegate (
    root:          TokenStream,
    generics:      TokenStream,
    dispatch_from: TokenStream,
    interface:     TokenStream,
    udata:         TokenStream,
    dispatch_to:   TokenStream
) -> TokenStream {
    quote! {
        impl #generics #root:Dispatch<#interface, #udata> for #dispatch_from {
            fn request (
                state:     &mut Self,
                client:    &$crate::Client,
                resource:  &$interface,
                request:   <$interface as $crate::Resource>::Request,
                data:      &$udata,
                dhandle:   &$crate::DisplayHandle,
                data_init: &mut $crate::DataInit<'_, Self>,
            ) {
                <$dispatch_to as $crate::Dispatch<$interface, $udata, Self>>::request(
                    state, client, resource, request, data, dhandle, data_init
                )
            }
            fn destroyed (
                state:    &mut Self,
                client:   $crate::backend::ClientId,
                resource: $crate::backend::ObjectId,
                data:     &$udata
            ) {
                <$dispatch_to as $crate::Dispatch<$interface, $udata, Self>>::destroyed(
                    state, client, resource, data
                )
            }
        }
    }
}

pub fn delegate_global (
    root:          TokenStream,
    generics:      TokenStream,
    dispatch_from: TokenStream,
    interface:     TokenStream,
    udata:         TokenStream,
    dispatch_to:   TokenStream
) -> TokenStream {
    quote! {
        impl #generics #root::GlobalDispatch<#interface, #udata> for #dispatch_from {
            fn bind (
                state:       &mut Self,
                dhandle:     &$crate::DisplayHandle,
                client:      &$crate::Client,
                resource:    $crate::New<$interface>,
                global_data: &$udata,
                data_init:   &mut $crate::DataInit<'_, Self>,
            ) {
                <$dispatch_to as $crate::GlobalDispatch<$interface, $udata, Self>>::bind(
                    state, dhandle, client, resource, global_data, data_init
                )
            }
            fn can_view (
                client:      $crate::Client,
                global_data: &$udata
            ) -> bool {
                <$dispatch_to as $crate::GlobalDispatch<$interface, $udata, Self>>::can_view(
                    client, global_data
                )
            }
        }
    }
}

pub fn delegate_output (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = OutputManagerState;
    delegator(&[
        delegate_global(c, g, f, WlOutput, WlOutputData, t),
        delegate_global(c, g, f, ZxdgOutputManagerV1, (), t)
    ], &[
        delegate(c, g, f, WlOutput, OutputUserData, t),
        delegate(c, g, f, ZxdgOutputV1, XdgOutputUserData, t),
        delegate(c, g, f, ZxdgOutputManagerV1, (), t)
    ])
}

pub fn delegate_compositor (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = CompositorState;
    delegator(&[
        delegate_global(c, g, f, WlCompositor, (), t),
        delegate_global(c, g, f, WlSubcompositor, (), t),
    ], &[
        delegate(c, g, f, WlCompositor, (), t),
        delegate(c, g, f, WlSurface, SurfaceUserData, t),
        delegate(c, g, f, WlRegion, RegionUserData, t),
        delegate(c, g, f, WlCallback, (), t),
        delegate(c, g, f, WlSubcompositor, (), t),
        delegate(c, g, f, WlSubsurface, SubsurfaceUserData, t),
    ])
}

pub fn delegate_xdg_shell (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = XdgShellState;
    delegator(&[
        delegate_global(c, g, f, XdgWmBase, (), t);
    ], &[
        delegate(c, g, f, XdgWmBase, XdgWmBaseUserData, t);
        delegate(c, g, f, XdgPositioner, XdgPositionerUserData, t);
        delegate(c, g, f, XdgPopup, XdgShellSurfaceUserData, t);
        delegate(c, g, f, XdgSurface, XdgSurfaceUserData, t);
        delegate(c, g, f, XdgToplevel, XdgShellSurfaceUserData, t);
    ])
}

pub fn delegate_shm (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = ShmState;
    delegator(&[
        delegate_global(c, g, f, WlShm, (), t);
    ], &[
        delegate(c, g, f, WlShm, (), t);
        delegate(c, g, f, WlShmPool, ShmPoolUserData, t);
        delegate(c, g, f, WlBuffer, ShmBuferUserData, t);
    ])
}

pub fn delegate_dmabuf (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = DmabufState;
    delegator(&[
        delegate_global(c, g, f, ZwpLinuxDmabufV1, DmabufGlobalData, t);
    ], &[
        delegate(c, g, f, ZwpLinuxDmabufV1, DmabufData, t);
        delegate(c, g, f, ZwpLinuxBufferParamsV1, DmabufParamsData, t);
        delegate(c, g, f, WlBuffer, Dmabuf, t);
    ])
}

pub fn delegate_fractional_scale (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = FractionalScaleManagerState;
    delegator(&[
        delegate_global(c, g, f, WpFractionalScaleManagerV1, Logger, t);
    ], &[
        delegate(c, g, f, WpFractionalScaleManagerV1, Logger, t);
        delegate(c, g, f, WpFractionalScaleV1, WlSurface, t);
    ])
}

pub fn delegate_presentation (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = PresentationState;
    delegator(&[
        delegate_global(c, g, f, WpPresentation, u32, t);
    ], &[
        delegate(c, g, f, WpPresentation, u32, t);
        delegate(c, g, f, WpPresentationFeedback, (), PresentationFeedbackState);
    ])
}

pub fn delegate_seat (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = SeatState<f>;
    delegator(&[
        delegate_global(c, g, f, WlSeat, SeatGlobalData<f>, t);
    ], &[
        delegate(c, g, f, WlSeat, SeatUserData<f>, t);
        delegate(c, g, f, WlPointer, PointerUserData<f>, t);
        delegate(c, g, f, WlKeyboard, KeyboardUserData<f>, t);
        delegate(c, g, f, WlTouch, TouchUserData, t);
    ])
}

pub fn delegate_data_device (input: TokenStream) -> TokenStream {
    let c = Crate;
    let g = Generics;
    let f = DispatchFrom;
    let t = DataDeviceState;
    delegator(&[
        delegate_global(c, g, f, WlDataDeviceManager, (), t)
    ], &[
        delegate(c, g, f, WlDataDeviceManager, (), t),
        delegate(c, g, f, WlDataDevice, DataDeviceUserData, t),
        delegate(c, g, f, WlDataSource, DataSourceUserData, t)
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

