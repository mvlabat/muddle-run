use crate::bevy_megaui::{
    MegaUiTexture, WindowSize, MEGAUI_TEXTURE_RESOURCE_BINDING_NAME,
    MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME,
};
use bevy::{
    asset::{Assets, Handle},
    core::AsBytes,
    ecs::{Commands, IntoSystem, Local, Res, ResMut, Resources, System, World},
    render::{
        render_graph::{CommandQueue, Node, ResourceSlots, SystemNode},
        renderer::{
            BindGroupId, BufferId, BufferInfo, BufferUsage, RenderContext, RenderResourceBinding,
            RenderResourceBindings, RenderResourceContext,
        },
        texture,
        texture::TextureDescriptor,
    },
};

#[derive(Debug)]
pub struct MegaUiTextureNode {
    font_texture_handle: Handle<MegaUiTexture>,
    command_queue: CommandQueue,
}

impl MegaUiTextureNode {
    pub fn new(font_texture_handle: Handle<MegaUiTexture>) -> Self {
        MegaUiTextureNode {
            font_texture_handle,
            command_queue: Default::default(),
        }
    }
}

impl Node for MegaUiTextureNode {
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

impl SystemNode for MegaUiTextureNode {
    fn get_system(&self, commands: &mut Commands) -> Box<dyn System<Input = (), Output = ()>> {
        let system = texture_node_system.system();
        commands.insert_local_resource(
            system.id(),
            TextureNodeState {
                command_queue: self.command_queue.clone(),
                font_texture_handle: self.font_texture_handle.clone(),
                initialized: false,
            },
        );
        Box::new(system)
    }
}

#[derive(Debug, Default)]
pub struct TextureNodeState {
    command_queue: CommandQueue,
    font_texture_handle: Handle<MegaUiTexture>,
    initialized: bool,
}

pub fn texture_node_system(
    mut state: Local<TextureNodeState>,
    render_resource_context: Res<Box<dyn RenderResourceContext>>,
    megaui_texture_assets: Res<Assets<MegaUiTexture>>,
    // PERF: this write on RenderResourceAssignments will prevent this system from running in parallel
    // with other systems that do the same
    mut render_resource_bindings: ResMut<RenderResourceBindings>,
) {
    let render_resource_context = &**render_resource_context;

    if !state.initialized {
        let font_texture = megaui_texture_assets
            .get(state.font_texture_handle.clone())
            .unwrap();

        let texture_descriptor: TextureDescriptor = (&font_texture.texture).into();
        let texture_resource = render_resource_context.create_texture(texture_descriptor);
        let sampler_resource =
            render_resource_context.create_sampler(&font_texture.texture.sampler);

        render_resource_bindings.set(
            MEGAUI_TEXTURE_RESOURCE_BINDING_NAME,
            RenderResourceBinding::Texture(texture_resource),
        );
        render_resource_bindings.set(
            MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME,
            RenderResourceBinding::Sampler(sampler_resource),
        );
        state.initialized = true;
    }
}
