use super::prelude::*;

use smithay::{
    backend::input::{
        Event,
        KeyState,
        KeyboardKeyEvent
    },
    input::keyboard::{
        keysyms,
        KeyboardHandle
    },
};

/// Possible results of a keyboard action
#[derive(Debug)]
enum KeyAction {
    /// Quit the compositor
    Quit,
    /// Trigger a vt-switch
    VtSwitch(i32),
    /// run a command
    Run(String),
    /// Switch the current screen
    Screen(usize),
    ScaleUp,
    ScaleDown,
    /// Forward the key to the client
    Forward,
    /// Do nothing more
    None,
}

pub struct Keyboard {
    logger:   Logger,
    keyboard: KeyboardHandle<AppState>,
    hotkeys:  Vec<u32>,
}

impl Keyboard {

    pub fn new (logger: &Logger, keyboard: KeyboardHandle<AppState>) -> Self {
        Self {
            logger: logger.clone(),
            keyboard,
            hotkeys: vec![],
        }
    }

    pub fn on_key <B: InputBackend> (
        state: &mut AppState,
        index: usize,
        event: B::KeyboardKeyEvent
    ) {
        let key_code   = event.key_code();
        let key_state  = event.state();
        let serial     = SERIAL_COUNTER.next_serial();
        let log        = &state.logger;
        let time       = Event::time(&event);
        let hotkeys    = &mut state.keyboards[index].hotkeys;
        let mut action = KeyAction::None;
        debug!(state.logger, "key"; "keycode" => key_code, "state" => format!("{:?}", key_state));
        //self.keyboard.input((), keycode, state, serial, time, |state, modifiers, keysym| {
            //debug!(log, "keysym";
                //"state"  => format!("{:?}", state),
                //"mods"   => format!("{:?}", modifiers),
                //"keysym" => ::xkbcommon::xkb::keysym_get_name(keysym)
            //);
            //if let KeyState::Pressed = state {
                //action = if modifiers.ctrl && modifiers.alt && keysym == keysyms::KEY_BackSpace
                    //|| modifiers.logo && keysym == keysyms::KEY_q
                //{
                    //KeyAction::Quit
                //} else if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&keysym) {
                    //// VTSwicth
                    //KeyAction::VtSwitch((keysym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32)
                //} else if modifiers.logo && keysym == keysyms::KEY_Return {
                    //// run terminal
                    //KeyAction::Run("weston-terminal".into())
                //} else if modifiers.logo && keysym >= keysyms::KEY_1 && keysym <= keysyms::KEY_9 {
                    //KeyAction::Screen((keysym - keysyms::KEY_1) as usize)
                //} else if modifiers.logo && modifiers.shift && keysym == keysyms::KEY_M {
                    //KeyAction::ScaleDown
                //} else if modifiers.logo && modifiers.shift && keysym == keysyms::KEY_P {
                    //KeyAction::ScaleUp
                //} else {
                    //KeyAction::Forward
                //};
                //// forward to client only if action == KeyAction::Forward
                //let forward = matches!(action, KeyAction::Forward);
                //if !forward { hotkeys.push(keysym); }
                //forward
            //} else {
                //let suppressed = hotkeys.contains(&keysym);
                //if suppressed { hotkeys.retain(|k| *k != keysym); }
                ////!suppressed
            //}
        //});

        //match action {
            //KeyAction::None | KeyAction::Forward => {}
            //KeyAction::Quit => {}
            //KeyAction::Run(cmd) => {}
            //KeyAction::ScaleUp => {}
            //KeyAction::ScaleDown => {}
            //action => {
                //warn!(self.logger, "Key action {:?} unsupported on winit backend.", action);
            //}
        //};
    }

}
