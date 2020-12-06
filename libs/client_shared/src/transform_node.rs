use crate::bevy_megaui::{MegaUiContext, WindowSize, MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME};
use bevy::{
    core::AsBytes,
    ecs::{Commands, IntoSystem, Local, Query, Res, ResMut, Resources, System, World},
    render::{
        render_graph::{CommandQueue, Node, ResourceSlots, SystemNode},
        renderer::{
            BufferId, BufferInfo, BufferUsage, RenderContext, RenderResourceBinding,
            RenderResourceBindings, RenderResourceContext,
        },
    },
};
use std::borrow::Cow;

#[derive(Debug)]
pub struct MegaUiTransformNode {
    command_queue: CommandQueue,
}

impl MegaUiTransformNode {
    pub fn new() -> Self {
        MegaUiTransformNode {
            command_queue: Default::default(),
        }
    }
}

impl Node for MegaUiTransformNode {
    fn update(
        &mut self,
        _world: &World,
        _resources: &Resources,
        render_context: &mut dyn RenderContext,
        _input: &ResourceSlots,
        _output: &mut ResourceSlots,
    ) {
        self.command_queue.execute(render_context);
    }
}

impl SystemNode for MegaUiTransformNode {
    fn get_system(&self, commands: &mut Commands) -> Box<dyn System<Input = (), Output = ()>> {
        let system = transform_node_system.system();
        commands.insert_local_resource(
            system.id(),
            TransformNodeState {
                command_queue: self.command_queue.clone(),
                transform_buffer: None,
                staging_buffer: None,
                prev_window_size: WindowSize::new(0.0, 0.0),
            },
        );
        Box::new(system)
    }
}

#[derive(Debug, Default)]
pub struct TransformNodeState {
    command_queue: CommandQueue,
    transform_buffer: Option<BufferId>,
    staging_buffer: Option<BufferId>,
    prev_window_size: WindowSize,
}

pub fn transform_node_system(
    mut state: Local<TransformNodeState>,
    render_resource_context: Res<Box<dyn RenderResourceContext>>,
    window_size: Res<WindowSize>,
    // PERF: this write on RenderResourceAssignments will prevent this system from running in parallel
    // with other systems that do the same
    mut render_resource_bindings: ResMut<RenderResourceBindings>,
) {
    let render_resource_context = &**render_resource_context;

    let staging_buffer = if let Some(staging_buffer) = state.staging_buffer {
        render_resource_context.map_buffer(staging_buffer);
        staging_buffer
    } else {
        let size = std::mem::size_of::<[[f32; 4]; 4]>();
        let buffer = render_resource_context.create_buffer(BufferInfo {
            size,
            buffer_usage: BufferUsage::COPY_DST | BufferUsage::UNIFORM,
            ..Default::default()
        });
        render_resource_bindings.set(
            MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME,
            RenderResourceBinding::Buffer {
                buffer,
                range: 0..size as u64,
                dynamic_index: None,
            },
        );
        state.transform_buffer = Some(buffer);

        let staging_buffer = render_resource_context.create_buffer(BufferInfo {
            size,
            buffer_usage: BufferUsage::COPY_SRC | BufferUsage::MAP_WRITE,
            mapped_at_creation: true,
        });

        state.staging_buffer = Some(staging_buffer);
        staging_buffer
    };

    let transform_data_size = std::mem::size_of::<[[f32; 2]; 2]>();
    let transform_data: [f32; 4] = [
        0.0,
        0.0, // transform
        1.0 / window_size.width,
        1.0 / window_size.height, // scale
    ];

    render_resource_context.write_mapped_buffer(
        staging_buffer,
        0..transform_data_size as u64,
        &mut |data, _renderer| {
            data[0..transform_data_size].copy_from_slice(transform_data.as_bytes());
        },
    );
    render_resource_context.unmap_buffer(staging_buffer);

    let transform_buffer = state.transform_buffer.unwrap();
    state.command_queue.copy_buffer_to_buffer(
        staging_buffer,
        0,
        transform_buffer,
        0,
        transform_data_size as u64,
    );
}
