extern crate proc_macro;
use proc_macro::TokenStream;

mod impls;
use crate::impls::{delegate, delegate_global};

macro_rules! delegator {
    ($name:ident) => {
        #[proc_macro_attribute]
        pub fn $name (input: TokenStream, _: TokenStream) -> TokenStream {
            crate::impls::$name(input.into()).into()
        }
    }
}

delegator!(delegate_output);

delegator!(delegate_compositor);

delegator!(delegate_xdg_shell);

delegator!(delegate_shm);

delegator!(delegate_dmabuf);

delegator!(delegate_fractional_scale);

delegator!(delegate_presentation);

delegator!(delegate_seat);

delegator!(delegate_data_device);

delegator!(delegate_keyboard_shortcuts_inhibit);

delegator!(delegate_layer_shell);

delegator!(delegate_viewporter);

delegator!(delegate_primary_selection);

delegator!(delegate_input_method_manager);

delegator!(delegate_tablet_manager);

delegator!(delegate_text_input_manager);

delegator!(delegate_virtual_keyboard_manager);

delegator!(delegate_xdg_activation);

delegator!(delegate_xdg_decoration);

delegator!(delegate_kde_decoration);
