use bevy::ecs::component::Component;

/// If an entity has this component, it'll be visible only if debug UI is shown.
#[derive(Component)]
pub struct DebugUiVisibility;
