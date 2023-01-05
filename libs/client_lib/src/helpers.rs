use crate::{
    input::{MouseRay, MouseScreenPosition},
    CurrentPlayerNetId, MainCameraEntity,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::{ReadOnlyWorldQuery, WorldQuery},
        system::{Local, Query, Res, SystemParam},
    },
    input::{mouse::MouseButton, Input},
    math::{Mat4, Vec2, Vec4},
    time::Time,
    utils::Instant,
    window::Window,
};
use mr_shared_lib::player::{Player, Players};
use std::{marker::PhantomData, time::Duration};

/// Radius in screen coordinates.
const DRAGGING_THRESHOLD: f32 = 10.0;
const DOUBLE_CLICK_MAX_DELAY_SECS: f64 = 0.3;

#[derive(SystemParam)]
pub struct PlayerParams<'w, 's> {
    pub players: Res<'w, Players>,
    pub current_player_net_id: Res<'w, CurrentPlayerNetId>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

impl<'w, 's> PlayerParams<'w, 's> {
    pub fn current_player(&self) -> Option<&Player> {
        self.current_player_net_id
            .0
            .and_then(|net_id| self.players.get(&net_id))
    }
}

#[derive(Clone, Default)]
pub struct Previous;
#[derive(Clone, Default)]
pub struct Current;

#[derive(SystemParam)]
pub struct MouseEntityPicker<'w, 's, Q: Send + Sync + 'static, F: Send + Sync + 'static> {
    prev_state: Local<'s, MouseEntityPickerData<Previous>>,
    state: Local<'s, MouseEntityPickerData<Current>>,
    button_input: Res<'w, Input<MouseButton>>,
    mouse_screen_position: Res<'w, MouseScreenPosition>,
    camera_query: Query<'w, 's, &'static bevy_mod_picking::PickingCamera>,
    camera_entity: Res<'w, MainCameraEntity>,
    time: Res<'w, Time>,
    #[system_param(ignore)]
    _q: std::marker::PhantomData<(Q, F)>,
}

#[derive(Clone, Default)]
pub struct MouseEntityPickerData<T> {
    /// Hovered entity will never equal picked entity, to make it possible to
    /// highlight an object underneath, for instance, while dragging another
    /// one (not currently used though).
    pub hovered_entity: Option<Entity>,
    pub picked_entity: Option<Entity>,
    pub dragging_start_position: Vec2,
    pub is_dragged: bool,
    pub is_just_picked: bool,
    pub is_just_clicked: bool,
    pub is_just_double_clicked: bool,
    pub hovered_at: Duration,
    pub picked_at: Duration,
    pub clicked_at: Duration,
    _t: std::marker::PhantomData<T>,
}

impl From<MouseEntityPickerData<Current>> for MouseEntityPickerData<Previous> {
    fn from(data: MouseEntityPickerData<Current>) -> Self {
        MouseEntityPickerData {
            hovered_entity: data.hovered_entity,
            picked_entity: data.picked_entity,
            dragging_start_position: data.dragging_start_position,
            is_dragged: data.is_dragged,
            is_just_picked: data.is_just_picked,
            is_just_clicked: data.is_just_clicked,
            is_just_double_clicked: data.is_just_double_clicked,
            hovered_at: data.hovered_at,
            picked_at: data.picked_at,
            clicked_at: data.clicked_at,
            _t: Default::default(),
        }
    }
}

impl<'w, 's, Q, F> MouseEntityPicker<'w, 's, Q, F>
where
    Q: WorldQuery + Send + Sync + 'static,
    F: ReadOnlyWorldQuery + Send + Sync + 'static,
{
    pub fn hovered_entity(&self, filter_query: &mut Option<&mut Query<Q, F>>) -> Option<Entity> {
        let picking_camera = self.camera_query.get(self.camera_entity.0).unwrap();
        picking_camera
            .intersections()
            .iter()
            .map(|(entity, _)| entity)
            .cloned()
            .filter(|entity| {
                filter_query
                    .as_mut()
                    .map_or(true, |query| query.get_mut(*entity).is_ok())
            })
            .find(|entity| Some(*entity) != self.state.picked_entity)
    }

    /// If an entity can be changed due to re-creating it because of a network
    /// update, this function needs to be called before `process_input`.
    pub fn update_entities(&mut self, f: impl Fn(Entity) -> Option<Entity>) {
        if let Some(hovered_entity) = self.state.hovered_entity {
            self.state.hovered_entity = f(hovered_entity);
        }
        if let Some(picked_entity) = self.state.picked_entity {
            self.state.picked_entity = f(picked_entity);
        }
    }

    pub fn process_input(&mut self, filter_query: &mut Option<&mut Query<Q, F>>) {
        *self.prev_state = (*self.state).clone().into();
        let now = self
            .time
            .last_update()
            .map_or_else(Instant::now, |last_update| last_update + self.time.delta())
            .duration_since(self.time.startup());

        // Updating "hovered" state.
        self.state.hovered_entity = self.hovered_entity(filter_query);
        if self.state.hovered_entity.is_some()
            && self.state.hovered_entity != self.prev_state.hovered_entity
        {
            self.state.hovered_at = now;
        }

        // Updating "clicked" and "picked" state.
        if self.prev_state.is_just_picked
            && now.saturating_sub(self.prev_state.picked_at)
                > Duration::from_secs_f64(DOUBLE_CLICK_MAX_DELAY_SECS)
        {
            self.state.is_just_picked = false;
        }
        if self.prev_state.is_just_clicked
            && now.saturating_sub(self.prev_state.clicked_at)
                > Duration::from_secs_f64(DOUBLE_CLICK_MAX_DELAY_SECS)
        {
            self.state.is_just_clicked = false;
        }
        if self.button_input.just_pressed(MouseButton::Left) {
            // We reset picked entity to allow `hovered_entity` to pick it up once more.
            self.state.picked_entity = None;
            self.state.picked_entity = self.hovered_entity(filter_query);
            if self.state.picked_entity.is_some() {
                self.state.dragging_start_position = self.mouse_screen_position.0;
                self.state.is_just_clicked = true;
                self.state.clicked_at = now;
            }

            if self.prev_state.is_just_clicked {
                self.state.is_just_double_clicked = true;
            }
        }

        if self.state.picked_entity != self.prev_state.picked_entity {
            // This is the point when prev_state can become totally different from the
            // current one.
            *self.state = MouseEntityPickerData {
                // Hovered entity should never be equal to picked entity.
                // (See documentation for `hovered_entity`.)
                hovered_entity: if self.state.hovered_entity == self.state.picked_entity {
                    None
                } else {
                    self.state.hovered_entity
                },
                picked_entity: self.state.picked_entity,
                dragging_start_position: self.state.dragging_start_position,
                is_dragged: false,
                is_just_picked: self.state.picked_entity.is_some(),
                is_just_clicked: self.state.picked_entity.is_some(),
                is_just_double_clicked: false,
                hovered_at: self.state.hovered_at,
                picked_at: now,
                clicked_at: now,
                _t: Default::default(),
            };
        }

        // Process dragging.
        if self.button_input.pressed(MouseButton::Left) {
            if self.state.picked_entity.is_some() && !self.state.is_dragged {
                self.state.is_dragged = self
                    .state
                    .dragging_start_position
                    .distance_squared(self.mouse_screen_position.0)
                    > DRAGGING_THRESHOLD * DRAGGING_THRESHOLD;
            }
        } else {
            self.state.is_dragged = false;
        }
    }

    pub fn picked_entity(&self) -> Option<Entity> {
        self.state.picked_entity
    }

    pub fn prev_state(&self) -> &MouseEntityPickerData<Previous> {
        &self.prev_state
    }

    pub fn state(&self) -> &MouseEntityPickerData<Current> {
        &self.state
    }

    pub fn reset(&mut self) {
        *self.state = Default::default();
        *self.prev_state = Default::default();
    }
}

// Heavily inspired by https://github.com/bevyengine/bevy/pull/432/.
pub fn cursor_pos_to_ray(
    cursor_viewport: Vec2,
    window: &Window,
    camera_transform: &Mat4,
    camera_perspective: &Mat4,
) -> MouseRay {
    // Calculate the cursor pos in NDC space [(-1,-1), (1,1)].
    let cursor_ndc = Vec4::from((
        (cursor_viewport.x / window.width()) * 2.0 - 1.0,
        (cursor_viewport.y / window.height()) * 2.0 - 1.0,
        -1.0, // let the cursor be on the far clipping plane
        1.0,
    ));

    let object_to_world = camera_transform;
    let object_to_ndc = camera_perspective;

    // Transform the cursor position into object/camera space. This also turns the
    // cursor into a vector that's pointing from the camera center onto the far
    // plane.
    let mut ray_camera = object_to_ndc.inverse().mul_vec4(cursor_ndc);
    ray_camera.z = -1.0;
    ray_camera.w = 0.0; // treat the vector as a direction (0 = Direction, 1 = Position)

    // Transform the cursor into world space.
    let ray_world = object_to_world.mul_vec4(ray_camera);
    let ray_world = ray_world.truncate();

    MouseRay {
        origin: camera_transform.w_axis.truncate(),
        direction: ray_world.normalize(),
    }
}
