use crate::prelude::*;

use super::{WinitEngine, WinitHostWindow, WinitHostError};

use smithay::{
    backend::{
        input::InputEvent,
        winit::{
            WinitInput,
            WinitEvent,
            WinitVirtualDevice,
            WinitKeyboardInputEvent,
            WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
            WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
        }
    },
    reexports::{
        winit::{
            event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
            event_loop::ControlFlow,
            platform::run_return::EventLoopExtRunReturn,
        }
    }
};

type ScreenId = usize;

pub type WinitUpdateContext = (
    InputEvent<WinitInput>,
    ScreenId
);

impl Update<WinitUpdateContext> for WinitEngine {
    fn update (&mut self, mut callback: WinitUpdateContext) -> StdResult<()> {
        let mut closed = false;
        if self.started.is_none() {
            let event = InputEvent::DeviceAdded { device: WinitVirtualDevice };
            callback(0, WinitEvent::Input(event));
            self.started = Some(Instant::now());
        }
        let started = &self.started.unwrap();
        let logger  = &self.logger;
        let outputs = &mut self.outputs;
        self.events.run_return(move |event, _target, control_flow| {
            //debug!(self.logger, "{target:?}");
            match event {
                Event::RedrawEventsCleared => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::RedrawRequested(_id) => {
                    callback(0, WinitEvent::Refresh);
                }
                Event::WindowEvent { window_id, event } => match outputs.get_mut(&window_id) {
                    Some(window) => {
                        window.update((started, event, &mut callback));
                        if window.closing {
                            outputs.remove(&window_id);
                            closed = true;
                        }
                    },
                    None => {
                        warn!(logger, "Received event for unknown window id {window_id:?}")
                    }
                }
                _ => {}
            }
        });
        if closed {
            Err(WinitHostError::WindowClosed.into())
        } else {
            Ok(())
        }
    }
}

impl<'a, T> Update<(&'a Instant, WindowEvent<'a>, &'a mut T)> for WinitHostWindow
where
    T: FnMut(ScreenId, WinitEvent)
{
    /// Dispatch input events from the host window to the hosted compositor.
    fn update (&mut self, (started, event, callback): (&'a Instant, WindowEvent<'a>, &'a mut T))
        -> StdResult<()>
    {
        //debug!(self.logger, "Winit Window Event: {self:?} {event:?}");
        let duration = Instant::now().duration_since(*started);
        let nanos = duration.subsec_nanos() as u64;
        let time = ((1000 * duration.as_secs()) + (nanos / 1_000_000)) as u32;
        Ok(match event {

            WindowEvent::Resized(psize) => {
                trace!(self.logger, "Resizing window to {:?}", psize);
                let scale_factor = self.window.scale_factor();
                let mut wsize    = self.size.borrow_mut();
                let (pw, ph): (u32, u32) = psize.into();
                wsize.physical_size = (pw as i32, ph as i32).into();
                wsize.scale_factor  = scale_factor;
                self.resized.set(Some(wsize.physical_size));
                callback(self.screen, WinitEvent::Resized {
                    size: wsize.physical_size,
                    scale_factor,
                });
            }

            WindowEvent::Focused(focus) => {
                callback(self.screen, WinitEvent::Focus(focus));
            }

            WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, } => {
                let mut wsize = self.size.borrow_mut();
                wsize.scale_factor = scale_factor;
                let (pw, ph): (u32, u32) = (*new_inner_size).into();
                self.resized.set(Some((pw as i32, ph as i32).into()));
                callback(self.screen, WinitEvent::Resized {
                    size: (pw as i32, ph as i32).into(),
                    scale_factor: wsize.scale_factor,
                });
            }

            WindowEvent::KeyboardInput { input, .. } => {
                let KeyboardInput { scancode, state, .. } = input;
                match state {
                    ElementState::Pressed => self.rollover += 1,
                    ElementState::Released => {
                        self.rollover = self.rollover.checked_sub(1).unwrap_or(0)
                    }
                };
                callback(self.screen, WinitEvent::Input(InputEvent::Keyboard {
                    event: WinitKeyboardInputEvent {
                        time, key: scancode, count: self.rollover, state,
                    },
                }));
            }

            WindowEvent::CursorMoved { position, .. } => {
                let lpos = position.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::PointerMotionAbsolute {
                    event: WinitMouseMovedEvent {
                        size: self.size.clone(), time, logical_position: lpos,
                    },
                }));
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let event = WinitMouseWheelEvent { time, delta };
                callback(self.screen, WinitEvent::Input(InputEvent::PointerAxis { event }));
            }

            WindowEvent::MouseInput { state, button, .. } => {
                callback(self.screen, WinitEvent::Input(InputEvent::PointerButton {
                    event: WinitMouseInputEvent {
                        time, button, state, is_x11: self.is_x11,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Started, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchDown {
                    event: WinitTouchStartedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Moved, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Ended, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
                callback(self.screen, WinitEvent::Input(InputEvent::TouchUp {
                    event: WinitTouchEndedEvent { time, id },
                }))
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Cancelled, id, .. }) => {
                callback(self.screen, WinitEvent::Input(InputEvent::TouchCancel {
                    event: WinitTouchCancelledEvent { time, id },
                }));
            }

            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                callback(self.screen, WinitEvent::Input(InputEvent::DeviceRemoved {
                    device: WinitVirtualDevice,
                }));
                warn!(self.logger, "Window closed");
                self.closing = true;
            }

            _ => {}

        })
    }

}
