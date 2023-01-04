use crate::prelude::*;
use crate::surface::{SurfaceData, SurfaceKind, draw_surface_tree};
use crate::popup::{Popup, PopupKind};
use crate::grab::{ResizeData, ResizeState, ResizeEdge};

struct Window {
    pub location: Point<i32, Logical>,
    /// A bounding box over this window and its children.
    ///
    /// Used for the fast path of the check in `matching`, and as the fall-back for the window
    /// geometry if that's not set explicitly.
    pub bbox: Rectangle<i32, Logical>,
    pub toplevel: SurfaceKind,
}

impl Window {
    /// Finds the topmost surface under this point if any and returns it together with the location of this
    /// surface.
    fn matching(&self, point: Point<f64, Logical>) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        if !self.bbox.to_f64().contains(point) {
            return None;
        }
        // need to check more carefully
        let found = RefCell::new(None);
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                self.location,
                |wl_surface, states, location| {
                    let mut location = *location;
                    let data = states.data_map.get::<RefCell<SurfaceData>>();

                    if states.role == Some("subsurface") {
                        let current = states.cached_state.current::<SubsurfaceCachedState>();
                        location += current.location;
                    }

                    let contains_the_point = data
                        .map(|data| {
                            data.borrow()
                                .contains_point(&*states.cached_state.current(), point - location.to_f64())
                        })
                        .unwrap_or(false);
                    if contains_the_point {
                        *found.borrow_mut() = Some((wl_surface.clone(), location));
                    }

                    TraversalAction::DoChildren(location)
                },
                |_, _, _| {},
                |_, _, _| {
                    // only continue if the point is not found
                    found.borrow().is_none()
                },
            );
        }
        found.into_inner()
    }

    fn self_update(&mut self) {
        let mut bounding_box = Rectangle::from_loc_and_size(self.location, (0, 0));
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                self.location,
                |_, states, &loc| {
                    let mut loc = loc;
                    let data = states.data_map.get::<RefCell<SurfaceData>>();

                    if let Some(size) = data.and_then(|d| d.borrow().size()) {
                        if states.role == Some("subsurface") {
                            let current = states.cached_state.current::<SubsurfaceCachedState>();
                            loc += current.location;
                        }

                        // Update the bounding box.
                        bounding_box = bounding_box.merge(Rectangle::from_loc_and_size(loc, size));

                        TraversalAction::DoChildren(loc)
                    } else {
                        // If the parent surface is unmapped, then the child surfaces are hidden as
                        // well, no need to consider them here.
                        TraversalAction::SkipChildren
                    }
                },
                |_, _, _| {},
                |_, _, _| true,
            );
        }
        self.bbox = bounding_box;
    }

    /// Returns the geometry of this window.
    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        // It's the set geometry with the full bounding box as the fallback.
        with_states(self.toplevel.get_surface().unwrap(), |states| {
            states.cached_state.current::<SurfaceCachedState>().geometry
        })
        .unwrap()
        .unwrap_or(self.bbox)
    }

    /// Sends the frame callback to all the subsurfaces in this
    /// window that requested it
    pub fn send_frame(&self, time: u32) {
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                (),
                |_, _, &()| TraversalAction::DoChildren(()),
                |_, states, &()| {
                    // the surface may not have any user_data if it is a subsurface and has not
                    // yet been commited
                    SurfaceData::send_frame(&mut *states.cached_state.current(), time)
                },
                |_, _, &()| true,
            );
        }
    }
}

#[derive(Default)]
pub struct WindowMap {
    windows: Vec<Window>,
    popups: Vec<Popup>,
}

impl WindowMap {
    pub fn insert(&mut self, toplevel: SurfaceKind, location: Point<i32, Logical>) {
        let mut window = Window {location, bbox: Rectangle::default(), toplevel};
        window.self_update();
        self.windows.insert(0, window);
    }

    pub fn windows(&self) -> impl Iterator<Item = SurfaceKind> + '_ {
        self.windows.iter().map(|w| w.toplevel.clone())
    }

    pub fn insert_popup(&mut self, popup: PopupKind) {
        let popup = Popup { popup };
        self.popups.push(popup);
    }

    pub fn get_surface_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        for w in &self.windows {
            if let Some(surface) = w.matching(point) {
                return Some(surface);
            }
        }
        None
    }

    pub fn get_surface_and_bring_to_top(
        &mut self,
        point: Point<f64, Logical>,
    ) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        let mut found = None;
        for (i, w) in self.windows.iter().enumerate() {
            if let Some(surface) = w.matching(point) {
                found = Some((i, surface));
                break;
            }
        }
        if let Some((i, surface)) = found {
            let winner = self.windows.remove(i);

            // Take activation away from all the windows
            for window in self.windows.iter() {
                window.toplevel.set_activated(false);
            }

            // Give activation to our winner
            winner.toplevel.set_activated(true);

            self.windows.insert(0, winner);
            Some(surface)
        } else {
            None
        }
    }

    pub fn with_windows_from_bottom_to_top<Func>(&self, mut f: Func)
    where
        Func: FnMut(&SurfaceKind, Point<i32, Logical>, &Rectangle<i32, Logical>),
    {
        for w in self.windows.iter().rev() {
            f(&w.toplevel, w.location, &w.bbox)
        }
    }
    pub fn with_child_popups<Func>(&self, base: &wl_surface::WlSurface, mut f: Func)
    where
        Func: FnMut(&PopupKind),
    {
        for w in self
            .popups
            .iter()
            .rev()
            .filter(move |w| w.popup.parent().as_ref() == Some(base))
        {
            f(&w.popup)
        }
    }

    pub fn refresh(&mut self) {
        self.windows.retain(|w| w.toplevel.alive());
        self.popups.retain(|p| p.popup.alive());
        for w in &mut self.windows {
            w.self_update();
        }
    }

    /// Refreshes the state of the toplevel, if it exists.
    pub fn refresh_toplevel(&mut self, toplevel: &SurfaceKind) {
        if let Some(w) = self.windows.iter_mut().find(|w| &w.toplevel == toplevel) {
            w.self_update();
        }
    }

    pub fn clear(&mut self) {
        self.windows.clear();
    }

    /// Finds the toplevel corresponding to the given `WlSurface`.
    pub fn find(&self, surface: &wl_surface::WlSurface) -> Option<SurfaceKind> {
        self.windows.iter().find_map(|w| {
            if w.toplevel
                .get_surface()
                .map(|s| s.as_ref().equals(surface.as_ref()))
                .unwrap_or(false)
            {
                Some(w.toplevel.clone())
            } else {
                None
            }
        })
    }

    /// Finds the popup corresponding to the given `WlSurface`.
    pub fn find_popup(&self, surface: &wl_surface::WlSurface) -> Option<PopupKind> {
        self.popups.iter().find_map(|p| {
            if p.popup
                .get_surface()
                .map(|s| s.as_ref().equals(surface.as_ref()))
                .unwrap_or(false)
            {
                Some(p.popup.clone())
            } else {
                None
            }
        })
    }

    /// Returns the location of the toplevel, if it exists.
    pub fn location(&self, toplevel: &SurfaceKind) -> Option<Point<i32, Logical>> {
        self.windows
            .iter()
            .find(|w| &w.toplevel == toplevel)
            .map(|w| w.location)
    }

    /// Sets the location of the toplevel, if it exists.
    pub fn set_location(&mut self, toplevel: &SurfaceKind, location: Point<i32, Logical>) {
        if let Some(w) = self.windows.iter_mut().find(|w| &w.toplevel == toplevel) {
            w.location = location;
            w.self_update();
        }
    }

    /// Returns the geometry of the toplevel, if it exists.
    pub fn geometry(&self, toplevel: &SurfaceKind) -> Option<Rectangle<i32, Logical>> {
        self.windows
            .iter()
            .find(|w| &w.toplevel == toplevel)
            .map(|w| w.geometry())
    }

    pub fn send_frames(&self, time: u32) {
        for window in &self.windows {
            window.send_frame(time);
        }
    }

    pub fn draw_windows<R, E, F, T>(
        &self,
        log: &Logger,
        renderer: &mut R,
        frame: &mut F,
        output_rect: Rectangle<i32, Logical>,
        output_scale: f32,
    ) -> Result<(), SwapBuffersError>
    where
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: std::error::Error + Into<SwapBuffersError>,
        T: Texture + 'static,
    {
        let mut result = Ok(());

        // redraw the frame, in a simple but inneficient way
        self.with_windows_from_bottom_to_top(|toplevel_surface, mut initial_place, &bounding_box| {
            // skip windows that do not overlap with a given output
            if !output_rect.overlaps(bounding_box) {
                return;
            }
            initial_place.x -= output_rect.loc.x;
            if let Some(wl_surface) = toplevel_surface.get_surface() {
                // this surface is a root of a subsurface tree that needs to be drawn
                if let Err(err) =
                    draw_surface_tree(log, renderer, frame, &wl_surface, initial_place, output_scale)
                {
                    result = Err(err);
                }
                // furthermore, draw its popups
                let toplevel_geometry_offset = self
                    .geometry(toplevel_surface)
                    .map(|g| g.loc)
                    .unwrap_or_default();
                self.with_child_popups(&wl_surface, |popup| {
                    let location = popup.location();
                    let draw_location = initial_place + location + toplevel_geometry_offset;
                    if let Some(wl_surface) = popup.get_surface() {
                        if let Err(err) = draw_surface_tree(
                            log, renderer, frame, &wl_surface, draw_location, output_scale
                        ) {
                            result = Err(err);
                        }
                    }
                });
            }
        });

        result
    }

    pub fn commit (&mut self, surface: &wl_surface::WlSurface) {
        #[cfg(feature = "xwayland")]
        super::xwayland::commit_hook(surface);
        if !is_sync_subsurface(surface) {
            // Update the buffer of all child surfaces
            with_surface_tree_upward(
                surface,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |_, states, _| {
                    states
                        .data_map
                        .insert_if_missing(|| RefCell::new(SurfaceData::default()));
                    let mut data = states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut();
                    data.update_buffer(&mut *states.cached_state.current::<SurfaceAttributes>());
                },
                |_, _, _| true,
            );
        }
        if let Some(toplevel) = self.find(surface) {
            // send the initial configure if relevant
            if let SurfaceKind::Xdg(ref toplevel) = toplevel {
                let initial_configure_sent = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<XdgToplevelSurfaceRoleAttributes>>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .initial_configure_sent
                })
                .unwrap();
                if !initial_configure_sent {
                    toplevel.send_configure();
                }
            }

            self.refresh_toplevel(&toplevel);

            let geometry = self.geometry(&toplevel).unwrap();
            let new_location = with_states(surface, |states| {
                let mut data = states
                    .data_map
                    .get::<RefCell<SurfaceData>>()
                    .unwrap()
                    .borrow_mut();

                let mut new_location = None;

                // If the window is being resized by top or left, its location must be adjusted
                // accordingly.
                match data.resize_state {
                    ResizeState::Resizing(resize_data)
                    | ResizeState::WaitingForFinalAck(resize_data, _)
                    | ResizeState::WaitingForCommit(resize_data) => {
                        let ResizeData {
                            edges,
                            initial_window_location,
                            initial_window_size,
                        } = resize_data;

                        if edges.intersects(ResizeEdge::TOP_LEFT) {
                            let mut location = self.location(&toplevel).unwrap();

                            if edges.intersects(ResizeEdge::LEFT) {
                                location.x =
                                    initial_window_location.x + (initial_window_size.w - geometry.size.w);
                            }
                            if edges.intersects(ResizeEdge::TOP) {
                                location.y =
                                    initial_window_location.y + (initial_window_size.h - geometry.size.h);
                            }

                            new_location = Some(location);
                        }
                    }
                    ResizeState::NotResizing => (),
                }

                // Finish resizing.
                if let ResizeState::WaitingForCommit(_) = data.resize_state {
                    data.resize_state = ResizeState::NotResizing;
                }

                new_location
            })
            .unwrap();

            if let Some(location) = new_location {
                self.set_location(&toplevel, location);
            }
        }

        if let Some(popup) = self.find_popup(surface) {
            let PopupKind::Xdg(ref popup) = popup;
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<XdgPopupSurfaceRoleAttributes>>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            })
            .unwrap();
            if !initial_configure_sent {
                // TODO: properly recompute the geometry with the whole of positioner state
                popup.send_configure();
            }
        }
    }
}
