use crate::transform_node::MegaUiTransformNode;
use bevy::{
    app::{stage, AppBuilder, EventReader, Events, Plugin},
    asset::{AssetEvent, Assets, Handle, HandleUntyped},
    core::{AsBytes, Time},
    ecs::{Resources, World},
    input::{keyboard::KeyCode, mouse::MouseButton, Input},
    reflect::TypeUuid,
    render::{
        pass::{
            ClearColor, LoadOp, Operations, PassDescriptor,
            RenderPassDepthStencilAttachmentDescriptor, TextureAttachment,
        },
        pipeline::{
            BindGroupDescriptor, BlendDescriptor, BlendFactor, BlendOperation,
            ColorStateDescriptor, ColorWrite, CompareFunction, CullMode,
            DepthStencilStateDescriptor, FrontFace, IndexFormat, InputStepMode, PipelineCompiler,
            PipelineDescriptor, PipelineLayout, PipelineSpecialization,
            RasterizationStateDescriptor, StencilStateDescriptor, StencilStateFaceDescriptor,
            VertexAttributeDescriptor, VertexBufferDescriptor, VertexFormat,
        },
        render_graph::{
            base, base::Msaa, Node, RenderGraph, ResourceSlotInfo, ResourceSlots,
            WindowSwapChainNode, WindowTextureNode,
        },
        renderer::{
            BindGroup, BindGroupId, BufferId, BufferInfo, BufferUsage, RenderContext,
            RenderResourceBinding, RenderResourceBindings, RenderResourceContext,
            RenderResourceType, SamplerId, TextureId,
        },
        shader::{Shader, ShaderStage, ShaderStages},
        texture::{Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat},
    },
    window::{CursorMoved, ReceivedCharacter, WindowResized, Windows},
};
use megaui::Vector2;
use std::{borrow::Cow, collections::HashMap};

pub const MEGAUI_PIPELINE_HANDLE: HandleUntyped =
    HandleUntyped::weak_from_u64(PipelineDescriptor::TYPE_UUID, 9404026720151354217);
pub const MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME: &str = "MegaUiTransform";
pub const MEGAUI_TEXTURE_RESOURCE_BINDING_NAME: &str = "MegaUiTexture_texture";

pub struct MegaUiPlugin;

pub struct MegaUiContext {
    pub ui: megaui::Ui,
    ui_draw_lists: Vec<megaui::DrawList>,
    font_texture: Handle<Texture>,
    megaui_textures: HashMap<u32, Handle<Texture>>,

    mouse_position: (f32, f32),
    cursor: EventReader<CursorMoved>,
    received_character: EventReader<ReceivedCharacter>,
    resize: EventReader<WindowResized>,
}

impl MegaUiContext {
    pub fn new(ui: megaui::Ui, font_texture: Handle<Texture>) -> Self {
        Self {
            ui,
            ui_draw_lists: Vec::new(),
            font_texture,
            megaui_textures: Default::default(),
            mouse_position: (0.0, 0.0),
            cursor: Default::default(),
            received_character: Default::default(),
            resize: Default::default(),
        }
    }

    /// A helper function to draw a megaui window.
    /// You may as well use `megaui::widgets::Window::new` if you prefer a builder pattern.
    pub fn draw_window(
        &mut self,
        id: megaui::Id,
        position: Vector2,
        size: Vector2,
        params: impl Into<Option<WindowParams>>,
        f: impl FnOnce(&mut megaui::Ui),
    ) {
        let params = params.into();

        megaui::widgets::Window::new(id, position, size)
            .label(params.as_ref().map_or("", |params| &params.label))
            .titlebar(params.as_ref().map_or(true, |params| params.titlebar))
            .movable(params.as_ref().map_or(true, |params| params.movable))
            .close_button(params.as_ref().map_or(false, |params| params.close_button))
            .ui(&mut self.ui, f);
    }

    /// Can accept either a strong or a weak handle.
    ///
    /// You may want to pass a weak handle if you control removing texture assets manually in
    /// your application and you don't want to bother with cleaning up textures in megaui.
    ///
    /// You'll want to pass a strong handle if a texture is used only in megaui and there's no
    /// handle copies stored anywhere else.
    pub fn set_megaui_texture(&mut self, id: u32, texture: Handle<Texture>) {
        log::debug!("Set megaui texture: {:?}", texture);
        self.megaui_textures.insert(id, texture);
    }

    /// Removes a texture handle associated with the id.
    pub fn remove_megaui_texture(&mut self, id: u32) {
        let texture_handle = self.megaui_textures.remove(&id);
        log::debug!("Remove megaui texture: {:?}", texture_handle);
    }

    // Is called when we get an event that a texture asset is removed.
    fn remove_texture(&mut self, texture_handle: &Handle<Texture>) {
        log::debug!("Removing megaui handles: {:?}", texture_handle);
        self.megaui_textures = self
            .megaui_textures
            .iter()
            .map(|(id, texture)| (*id, texture.clone()))
            .filter(|(_, texture)| texture != texture_handle)
            .collect();
    }
}

pub struct WindowParams {
    pub label: String,
    pub movable: bool,
    pub close_button: bool,
    pub titlebar: bool,
}

impl Default for WindowParams {
    fn default() -> WindowParams {
        WindowParams {
            label: "".to_string(),
            movable: true,
            close_button: false,
            titlebar: true,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
}

impl WindowSize {
    pub fn new(width: f32, height: f32, scale_factor: f32) -> Self {
        Self {
            width,
            height,
            scale_factor,
        }
    }
}

impl MegaUiContext {
    pub fn render_draw_lists(&mut self) {
        self.ui_draw_lists.clear();
        self.ui.render(&mut self.ui_draw_lists);
    }
}

pub mod node {
    pub const MEGAUI_PASS: &str = "megaui_pass";
    pub const MEGAUI_TRANSFORM: &str = "megaui_transform";
}

impl Plugin for MegaUiPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.add_system_to_stage(stage::PRE_UPDATE, process_input);

        let resources = app.resources_mut();

        let ui = megaui::Ui::new();
        let font_texture = {
            let mut assets = resources.get_mut::<Assets<Texture>>().unwrap();
            assets.add(Texture::new(
                Extent3d::new(ui.font_atlas.texture.width, ui.font_atlas.texture.height, 1),
                TextureDimension::D2,
                ui.font_atlas.texture.data.clone(),
                TextureFormat::Rgba8Unorm,
            ))
        };
        resources.insert(WindowSize::new(0.0, 0.0, 0.0));
        resources.insert_thread_local(MegaUiContext::new(ui, font_texture.clone()));

        let mut pipelines = resources.get_mut::<Assets<PipelineDescriptor>>().unwrap();
        let mut shaders = resources.get_mut::<Assets<Shader>>().unwrap();

        pipelines.set_untracked(MEGAUI_PIPELINE_HANDLE, build_megaui_pipeline(&mut shaders));
        let pipeline_descriptor_handle = {
            let render_resource_context =
                resources.get::<Box<dyn RenderResourceContext>>().unwrap();
            let mut pipeline_compiler = resources.get_mut::<PipelineCompiler>().unwrap();

            let attributes = vec![
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Position"),
                    offset: 0,
                    format: VertexFormat::Float3,
                    shader_location: 0,
                },
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Uv"),
                    offset: VertexFormat::Float3.get_size(),
                    format: VertexFormat::Float2,
                    shader_location: 1,
                },
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Color"),
                    offset: VertexFormat::Float3.get_size() + VertexFormat::Float2.get_size(),
                    format: VertexFormat::Float4,
                    shader_location: 2,
                },
            ];
            pipeline_compiler.compile_pipeline(
                render_resource_context.as_ref(),
                &mut pipelines,
                &mut shaders,
                &MEGAUI_PIPELINE_HANDLE.typed(),
                &PipelineSpecialization {
                    vertex_buffer_descriptor: VertexBufferDescriptor {
                        name: Cow::from("MegaUiVertex"),
                        stride: attributes
                            .iter()
                            .fold(0, |acc, attribute| acc + attribute.format.get_size()),
                        step_mode: InputStepMode::Vertex,
                        attributes,
                    },
                    index_format: IndexFormat::Uint16,
                    ..PipelineSpecialization::default()
                },
            )
        };
        let pipeline_descriptor = pipelines.get(pipeline_descriptor_handle.clone()).unwrap();
        let layout = pipeline_descriptor.layout.as_ref().unwrap();
        let transform_bind_group =
            find_bind_group_by_binding_name(layout, MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME)
                .unwrap();
        let texture_bind_group =
            find_bind_group_by_binding_name(layout, MEGAUI_TEXTURE_RESOURCE_BINDING_NAME).unwrap();

        let msaa = resources.get::<Msaa>().unwrap();

        let mut render_graph = resources.get_mut::<RenderGraph>().unwrap();

        render_graph.add_node(
            node::MEGAUI_PASS,
            MegaUiNode::new(
                pipeline_descriptor_handle,
                transform_bind_group,
                texture_bind_group,
                &msaa,
                font_texture,
            ),
        );
        render_graph
            .add_node_edge(base::node::MAIN_PASS, node::MEGAUI_PASS)
            .unwrap();

        render_graph
            .add_slot_edge(
                base::node::PRIMARY_SWAP_CHAIN,
                WindowSwapChainNode::OUT_TEXTURE,
                node::MEGAUI_PASS,
                if msaa.samples > 1 {
                    "color_resolve_target"
                } else {
                    "color_attachment"
                },
            )
            .unwrap();

        render_graph
            .add_slot_edge(
                base::node::MAIN_DEPTH_TEXTURE,
                WindowTextureNode::OUT_TEXTURE,
                node::MEGAUI_PASS,
                "depth",
            )
            .unwrap();

        if msaa.samples > 1 {
            render_graph
                .add_slot_edge(
                    base::node::MAIN_SAMPLED_COLOR_ATTACHMENT,
                    WindowSwapChainNode::OUT_TEXTURE,
                    node::MEGAUI_PASS,
                    "color_attachment",
                )
                .unwrap();
        }

        // Transform.
        render_graph.add_system_node(node::MEGAUI_TRANSFORM, MegaUiTransformNode::new());
        render_graph
            .add_node_edge(node::MEGAUI_TRANSFORM, node::MEGAUI_PASS)
            .unwrap();
    }
}

fn find_bind_group_by_binding_name(
    pipeline_layout: &PipelineLayout,
    binding_name: &str,
) -> Option<BindGroupDescriptor> {
    pipeline_layout
        .bind_groups
        .iter()
        .find(|bind_group| {
            bind_group
                .bindings
                .iter()
                .any(|binding| binding.name == binding_name)
        })
        .cloned()
}

pub struct MegaUiNode {
    pass_descriptor: PassDescriptor,
    pipeline_descriptor: Handle<PipelineDescriptor>,
    inputs: Vec<ResourceSlotInfo>,
    color_attachment_input_indices: Vec<Option<usize>>,
    color_resolve_target_indices: Vec<Option<usize>>,
    depth_stencil_attachment_input_index: Option<usize>,
    default_clear_color_inputs: Vec<usize>,

    transform_bind_group_descriptor: BindGroupDescriptor,
    transform_bind_group_id: Option<BindGroupId>,

    font_texture: Handle<Texture>,
    texture_bind_group_descriptor: BindGroupDescriptor,
    texture_resources: HashMap<Handle<Texture>, TextureResource>,
    event_reader: EventReader<AssetEvent<Texture>>,

    vertex_buffer: Option<BufferId>,
    index_buffer: Option<BufferId>,
}

#[derive(Debug)]
pub struct TextureResource {
    descriptor: TextureDescriptor,
    texture: TextureId,
    sampler: SamplerId,
    bind_group: BindGroupId,
}

impl MegaUiNode {
    pub fn new(
        pipeline_descriptor: Handle<PipelineDescriptor>,
        transform_bind_group_descriptor: BindGroupDescriptor,
        texture_bind_group_descriptor: BindGroupDescriptor,
        msaa: &Msaa,
        font_texture: Handle<Texture>,
    ) -> Self {
        let color_attachments = vec![msaa.color_attachment_descriptor(
            TextureAttachment::Input("color_attachment".to_string()),
            TextureAttachment::Input("color_resolve_target".to_string()),
            Operations {
                load: LoadOp::Load,
                store: true,
            },
        )];
        let depth_stencil_attachment = RenderPassDepthStencilAttachmentDescriptor {
            attachment: TextureAttachment::Input("depth".to_string()),
            depth_ops: Some(Operations {
                load: LoadOp::Clear(1.0),
                store: true,
            }),
            stencil_ops: None,
        };

        let mut inputs = Vec::new();
        let mut color_attachment_input_indices = Vec::new();
        let mut color_resolve_target_indices = Vec::new();

        for color_attachment in color_attachments.iter() {
            if let TextureAttachment::Input(ref name) = color_attachment.attachment {
                color_attachment_input_indices.push(Some(inputs.len()));
                inputs.push(ResourceSlotInfo::new(
                    name.to_string(),
                    RenderResourceType::Texture,
                ));
            } else {
                color_attachment_input_indices.push(None);
            }

            if let Some(TextureAttachment::Input(ref name)) = color_attachment.resolve_target {
                color_resolve_target_indices.push(Some(inputs.len()));
                inputs.push(ResourceSlotInfo::new(
                    name.to_string(),
                    RenderResourceType::Texture,
                ));
            } else {
                color_resolve_target_indices.push(None);
            }
        }

        let mut depth_stencil_attachment_input_index = None;
        if let TextureAttachment::Input(ref name) = depth_stencil_attachment.attachment {
            depth_stencil_attachment_input_index = Some(inputs.len());
            inputs.push(ResourceSlotInfo::new(
                name.to_string(),
                RenderResourceType::Texture,
            ));
        }

        Self {
            pass_descriptor: PassDescriptor {
                color_attachments,
                depth_stencil_attachment: Some(depth_stencil_attachment),
                sample_count: msaa.samples,
            },
            pipeline_descriptor,
            default_clear_color_inputs: Vec::new(),
            inputs,
            depth_stencil_attachment_input_index,
            color_attachment_input_indices,
            transform_bind_group_descriptor,
            transform_bind_group_id: None,
            font_texture,
            texture_bind_group_descriptor,
            texture_resources: Default::default(),
            event_reader: Default::default(),
            vertex_buffer: None,
            index_buffer: None,
            color_resolve_target_indices,
        }
    }
}

struct DrawCommand {
    vertices_count: usize,
    texture_handle: Option<Handle<Texture>>,
    clipping_zone: Option<megaui::Rect>,
}

impl Node for MegaUiNode {
    fn input(&self) -> &[ResourceSlotInfo] {
        &self.inputs
    }

    fn update(
        &mut self,
        _world: &World,
        resources: &Resources,
        render_context: &mut dyn RenderContext,
        input: &ResourceSlots,
        _output: &mut ResourceSlots,
    ) {
        self.process_attachments(input, resources);

        let window_size = resources.get::<WindowSize>().unwrap();

        let render_resource_bindings = resources.get::<RenderResourceBindings>().unwrap();

        self.init_transform_bind_group(render_context, &render_resource_bindings);

        let texture_assets = resources.get_mut::<Assets<Texture>>().unwrap();
        let asset_events = resources.get::<Events<AssetEvent<Texture>>>().unwrap();

        let mut megaui_context = resources.get_thread_local_mut::<MegaUiContext>().unwrap();

        self.process_asset_events(
            render_context,
            &mut megaui_context,
            &asset_events,
            &texture_assets,
        );
        self.init_textures(render_context, &megaui_context, &texture_assets);

        megaui_context.render_draw_lists();
        let mut ui_draw_lists = Vec::new();

        std::mem::swap(&mut ui_draw_lists, &mut megaui_context.ui_draw_lists);

        let mut vertex_buffer = Vec::<u8>::new();
        let mut index_buffer = Vec::new();
        let mut draw_commands = Vec::new();
        let mut index_offset = 0;

        for draw_list in &ui_draw_lists {
            let texture_handle = if let Some(texture) = draw_list.texture {
                megaui_context.megaui_textures.get(&texture).cloned()
            } else {
                Some(megaui_context.font_texture.clone())
            };

            for vertex in &draw_list.vertices {
                vertex_buffer.extend_from_slice(vertex.pos.as_bytes());
                vertex_buffer.extend_from_slice(vertex.uv.as_bytes());
                vertex_buffer.extend_from_slice(vertex.color.as_bytes());
            }
            let indices_with_offset = draw_list
                .indices
                .iter()
                .map(|i| i + index_offset)
                .collect::<Vec<_>>();
            index_buffer.extend_from_slice(indices_with_offset.as_slice().as_bytes());
            index_offset += draw_list.vertices.len() as u16;

            draw_commands.push(DrawCommand {
                vertices_count: draw_list.indices.len(),
                texture_handle,
                clipping_zone: draw_list.clipping_zone,
            });
        }

        self.update_buffers(render_context, &vertex_buffer, &index_buffer);

        render_context.begin_pass(
            &self.pass_descriptor,
            &render_resource_bindings,
            &mut |render_pass| {
                render_pass.set_pipeline(&self.pipeline_descriptor);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.unwrap(), 0);
                render_pass.set_index_buffer(self.index_buffer.unwrap(), 0);
                render_pass.set_bind_group(
                    0,
                    self.transform_bind_group_descriptor.id,
                    self.transform_bind_group_id.unwrap(),
                    None,
                );

                // This is a pretty weird kludge, but we need to bind all our groups at least once,
                // so they don't get garbage collected by `remove_stale_bind_groups`.
                for texture_resource in self.texture_resources.values() {
                    render_pass.set_bind_group(
                        1,
                        self.texture_bind_group_descriptor.id,
                        texture_resource.bind_group,
                        None,
                    );
                }

                let mut vertex_offset: u32 = 0;
                for draw_command in &draw_commands {
                    let texture_resource = match draw_command
                        .texture_handle
                        .as_ref()
                        .and_then(|texture_handle| self.texture_resources.get(texture_handle))
                    {
                        Some(texture_resource) => texture_resource,
                        None => {
                            vertex_offset += draw_command.vertices_count as u32;
                            continue;
                        }
                    };

                    render_pass.set_bind_group(
                        1,
                        self.texture_bind_group_descriptor.id,
                        texture_resource.bind_group,
                        None,
                    );

                    if let Some(clipping_zone) = draw_command.clipping_zone {
                        render_pass.set_scissor_rect(
                            (clipping_zone.x * window_size.scale_factor) as u32,
                            (clipping_zone.y * window_size.scale_factor) as u32,
                            (clipping_zone.w * window_size.scale_factor) as u32,
                            (clipping_zone.h * window_size.scale_factor) as u32,
                        );
                    } else {
                        render_pass.set_scissor_rect(
                            0,
                            0,
                            (window_size.width * window_size.scale_factor) as u32,
                            (window_size.height * window_size.scale_factor) as u32,
                        );
                    }
                    render_pass.draw_indexed(
                        vertex_offset..(vertex_offset + draw_command.vertices_count as u32),
                        0,
                        0..1,
                    );
                    vertex_offset += draw_command.vertices_count as u32;
                }
            },
        );

        std::mem::swap(&mut ui_draw_lists, &mut megaui_context.ui_draw_lists);
        megaui_context
            .ui
            .new_frame(resources.get::<Time>().unwrap().delta_seconds());
    }
}

impl MegaUiNode {
    fn process_attachments(&mut self, input: &ResourceSlots, resources: &Resources) {
        if let Some(input_index) = self.depth_stencil_attachment_input_index {
            self.pass_descriptor
                .depth_stencil_attachment
                .as_mut()
                .unwrap()
                .attachment =
                TextureAttachment::Id(input.get(input_index).unwrap().get_texture().unwrap());
        }

        for (i, color_attachment) in self
            .pass_descriptor
            .color_attachments
            .iter_mut()
            .enumerate()
        {
            if self.default_clear_color_inputs.contains(&i) {
                if let Some(default_clear_color) = resources.get::<ClearColor>() {
                    color_attachment.ops.load = LoadOp::Clear(default_clear_color.0);
                }
            }
            if let Some(input_index) = self.color_attachment_input_indices[i] {
                color_attachment.attachment =
                    TextureAttachment::Id(input.get(input_index).unwrap().get_texture().unwrap());
            }
            if let Some(input_index) = self.color_resolve_target_indices[i] {
                color_attachment.resolve_target = Some(TextureAttachment::Id(
                    input.get(input_index).unwrap().get_texture().unwrap(),
                ));
            }
        }
    }

    fn init_transform_bind_group(
        &mut self,
        render_context: &mut dyn RenderContext,
        render_resource_bindings: &RenderResourceBindings,
    ) {
        if self.transform_bind_group_id.is_none() {
            let transform_bindings = render_resource_bindings
                .get(MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME)
                .unwrap()
                .clone();
            let transform_bind_group = BindGroup::build()
                .add_binding(0, transform_bindings)
                .finish();
            render_context.resources().create_bind_group(
                self.transform_bind_group_descriptor.id,
                &transform_bind_group,
            );
            self.transform_bind_group_id = Some(transform_bind_group.id);
        }
    }

    fn process_asset_events(
        &mut self,
        render_context: &mut dyn RenderContext,
        megaui_context: &mut MegaUiContext,
        asset_events: &Events<AssetEvent<Texture>>,
        texture_assets: &Assets<Texture>,
    ) {
        let mut changed_assets: HashMap<Handle<Texture>, &Texture> = HashMap::new();
        for event in self.event_reader.iter(asset_events) {
            let handle = match event {
                AssetEvent::Created { ref handle }
                | AssetEvent::Modified { ref handle }
                | AssetEvent::Removed { ref handle } => handle,
            };
            if !self.texture_resources.contains_key(handle) {
                continue;
            }
            log::debug!("{:?}", event);

            match event {
                AssetEvent::Created { .. } => {
                    // Don't have to do anything really, since we track uninitialized textures
                    // via `MegaUiContext::set_megaui_texture` and `Self::init_textures`.
                }
                AssetEvent::Modified { ref handle } => {
                    if let Some(asset) = texture_assets.get(handle) {
                        changed_assets.insert(handle.clone(), asset);
                    }
                }
                AssetEvent::Removed { ref handle } => {
                    megaui_context.remove_texture(handle);
                    self.remove_texture(render_context, handle);
                    // If an asset was modified and removed in the same update, ignore the modification.
                    changed_assets.remove(&handle);
                }
            }
        }
        for (texture_handle, texture) in changed_assets {
            self.update_texture(render_context, texture, texture_handle);
        }
    }

    fn init_textures(
        &mut self,
        render_context: &mut dyn RenderContext,
        megaui_context: &MegaUiContext,
        texture_assets: &Assets<Texture>,
    ) {
        self.create_texture(render_context, texture_assets, self.font_texture.clone());

        for texture in megaui_context.megaui_textures.values() {
            self.create_texture(render_context, texture_assets, texture.clone_weak());
        }
    }

    fn update_texture(
        &mut self,
        render_context: &mut dyn RenderContext,
        texture_asset: &Texture,
        texture_handle: Handle<Texture>,
    ) {
        let texture_resource = match self.texture_resources.get(&texture_handle) {
            Some(texture_resource) => texture_resource,
            None => return,
        };
        log::debug!("Updating a texture: ${:?}", texture_handle);

        let texture_descriptor: TextureDescriptor = texture_asset.into();

        if texture_descriptor != texture_resource.descriptor {
            log::debug!(
                "Removing an updated texture for it to be re-created later: {:?}",
                texture_handle
            );
            // If a texture descriptor is updated, we'll re-create the texture in `init_textures`.
            self.remove_texture(render_context, &texture_handle);
            return;
        }
        Self::copy_texture(render_context, &texture_resource, texture_asset);
    }

    fn create_texture(
        &mut self,
        render_context: &mut dyn RenderContext,
        texture_assets: &Assets<Texture>,
        texture_handle: Handle<Texture>,
    ) {
        if self.texture_resources.contains_key(&texture_handle) {
            return;
        }

        // If a texture is still loading, we skip it.
        let texture_asset = match texture_assets.get(texture_handle.clone()) {
            Some(texture_asset) => texture_asset,
            None => return,
        };

        log::info!("Creating a texture: ${:?}", texture_handle);

        let render_resource_context = render_context.resources();

        let texture_descriptor: TextureDescriptor = texture_asset.into();
        let texture = render_resource_context.create_texture(texture_descriptor);
        let sampler = render_resource_context.create_sampler(&texture_asset.sampler);

        let texture_bind_group = BindGroup::build()
            .add_binding(0, RenderResourceBinding::Texture(texture))
            .add_binding(1, RenderResourceBinding::Sampler(sampler))
            .finish();

        render_resource_context
            .create_bind_group(self.texture_bind_group_descriptor.id, &texture_bind_group);

        let texture_resource = TextureResource {
            descriptor: texture_descriptor,
            texture,
            sampler,
            bind_group: texture_bind_group.id,
        };
        Self::copy_texture(render_context, &texture_resource, texture_asset);
        log::debug!("Texture created: {:?}", texture_resource);
        self.texture_resources
            .insert(texture_handle, texture_resource);
    }

    fn remove_texture(
        &mut self,
        render_context: &mut dyn RenderContext,
        texture_handle: &Handle<Texture>,
    ) {
        let texture_resource = match self.texture_resources.remove(texture_handle) {
            Some(texture_resource) => texture_resource,
            None => return,
        };
        log::debug!("Removing a texture: ${:?}", texture_handle);

        let render_resource_context = render_context.resources();
        render_resource_context.remove_texture(texture_resource.texture);
        render_resource_context.remove_sampler(texture_resource.sampler);
    }

    fn copy_texture(
        render_context: &mut dyn RenderContext,
        texture_resource: &TextureResource,
        texture: &Texture,
    ) {
        let aligned_width = render_context
            .resources()
            .get_aligned_texture_size(texture.size.width as usize);
        let format_size = texture.format.pixel_size();

        let texture_buffer = render_context.resources().create_buffer_with_data(
            BufferInfo {
                buffer_usage: BufferUsage::COPY_SRC,
                ..Default::default()
            },
            &texture.data,
        );

        render_context.copy_buffer_to_texture(
            texture_buffer,
            0,
            (format_size * aligned_width) as u32,
            texture_resource.texture,
            [0, 0, 0],
            0,
            texture_resource.descriptor.size,
        );
        render_context.resources().remove_buffer(texture_buffer);
    }

    fn update_buffers(
        &mut self,
        render_context: &mut dyn RenderContext,
        vertex_buffer: &[u8],
        index_buffer: &[u8],
    ) {
        if let Some(vertex_buffer) = self.vertex_buffer.take() {
            render_context.resources().remove_buffer(vertex_buffer);
        }
        if let Some(index_buffer) = self.index_buffer.take() {
            render_context.resources().remove_buffer(index_buffer);
        }
        self.vertex_buffer = Some(render_context.resources().create_buffer_with_data(
            BufferInfo {
                buffer_usage: BufferUsage::VERTEX,
                ..Default::default()
            },
            vertex_buffer,
        ));
        self.index_buffer = Some(render_context.resources().create_buffer_with_data(
            BufferInfo {
                buffer_usage: BufferUsage::INDEX,
                ..Default::default()
            },
            index_buffer,
        ));
    }
}

// Is a thread local system, because `megaui::Ui` (`MegaUiContext`) doesn't implement Send + Sync.
fn process_input(_world: &mut World, resources: &mut Resources) {
    use megaui::InputHandler;

    let mut ctx = resources.get_thread_local_mut::<MegaUiContext>().unwrap();
    let ev_cursor = resources.get::<Events<CursorMoved>>().unwrap();
    let ev_received_character = resources.get::<Events<ReceivedCharacter>>().unwrap();
    let ev_resize = resources.get::<Events<WindowResized>>().unwrap();
    let mouse_button_input = resources.get::<Input<MouseButton>>().unwrap();
    let keyboard_input = resources.get::<Input<KeyCode>>().unwrap();
    let mut window_size = resources.get_mut::<WindowSize>().unwrap();
    let windows = resources.get::<Windows>().unwrap();

    if *window_size == WindowSize::new(0.0, 0.0, 0.0) {
        let window = windows.get_primary().unwrap();
        *window_size = WindowSize::new(
            window.logical_width(),
            window.logical_height(),
            window.scale_factor() as f32,
        );
    }
    if let Some(resize_event) = ctx.resize.latest(&ev_resize) {
        let is_primary = windows
            .get_primary()
            .map_or(false, |window| window.id() == resize_event.id);
        if is_primary {
            window_size.width = resize_event.width;
            window_size.height = resize_event.height;
        }
    }

    if let Some(cursor_moved) = ctx.cursor.latest(&ev_cursor) {
        let mut mouse_position: (f32, f32) = cursor_moved.position.into();
        mouse_position.1 = window_size.height - mouse_position.1;
        ctx.mouse_position = mouse_position;
        ctx.ui.mouse_move(mouse_position);
    }

    let mouse_position = ctx.mouse_position;
    if mouse_button_input.just_pressed(MouseButton::Left) {
        ctx.ui.mouse_down(mouse_position);
    }
    if mouse_button_input.just_released(MouseButton::Left) {
        ctx.ui.mouse_up(mouse_position);
    }

    let shift = keyboard_input.pressed(KeyCode::LShift) || keyboard_input.pressed(KeyCode::RShift);
    let ctrl =
        keyboard_input.pressed(KeyCode::LControl) || keyboard_input.pressed(KeyCode::RControl);

    for event in ctx.received_character.iter(&ev_received_character) {
        if event.id.is_primary() && !event.char.is_control() {
            ctx.ui.char_event(event.char, shift, ctrl);
        }
    }

    if keyboard_input.pressed(KeyCode::Up) {
        ctx.ui.key_down(megaui::KeyCode::Up, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Down) {
        ctx.ui.key_down(megaui::KeyCode::Down, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Right) {
        ctx.ui.key_down(megaui::KeyCode::Right, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Left) {
        ctx.ui.key_down(megaui::KeyCode::Left, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Home) {
        ctx.ui.key_down(megaui::KeyCode::Home, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::End) {
        ctx.ui.key_down(megaui::KeyCode::End, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Delete) {
        ctx.ui.key_down(megaui::KeyCode::Delete, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Back) {
        ctx.ui.key_down(megaui::KeyCode::Backspace, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Return) {
        ctx.ui.key_down(megaui::KeyCode::Enter, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Tab) {
        ctx.ui.key_down(megaui::KeyCode::Tab, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Z) {
        ctx.ui.key_down(megaui::KeyCode::Z, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::Y) {
        ctx.ui.key_down(megaui::KeyCode::Y, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::C) {
        ctx.ui.key_down(megaui::KeyCode::C, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::X) {
        ctx.ui.key_down(megaui::KeyCode::X, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::V) {
        ctx.ui.key_down(megaui::KeyCode::V, shift, ctrl);
    }
    if keyboard_input.pressed(KeyCode::A) {
        ctx.ui.key_down(megaui::KeyCode::A, shift, ctrl);
    }
}

pub fn build_megaui_pipeline(shaders: &mut Assets<Shader>) -> PipelineDescriptor {
    PipelineDescriptor {
        rasterization_state: Some(RasterizationStateDescriptor {
            front_face: FrontFace::Cw,
            cull_mode: CullMode::None,
            depth_bias: 0,
            depth_bias_slope_scale: 0.0,
            depth_bias_clamp: 0.0,
            clamp_depth: false,
        }),
        depth_stencil_state: Some(DepthStencilStateDescriptor {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: CompareFunction::LessEqual,
            stencil: StencilStateDescriptor {
                front: StencilStateFaceDescriptor::IGNORE,
                back: StencilStateFaceDescriptor::IGNORE,
                read_mask: 0,
                write_mask: 0,
            },
        }),
        color_states: vec![ColorStateDescriptor {
            format: TextureFormat::default(),
            color_blend: BlendDescriptor {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha_blend: BlendDescriptor {
                src_factor: BlendFactor::OneMinusDstAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            write_mask: ColorWrite::ALL,
        }],
        index_format: IndexFormat::Uint16,
        ..PipelineDescriptor::new(ShaderStages {
            vertex: shaders.add(Shader::from_glsl(
                ShaderStage::Vertex,
                include_str!("megaui.vert"),
            )),
            fragment: Some(shaders.add(Shader::from_glsl(
                ShaderStage::Fragment,
                include_str!("megaui.frag"),
            ))),
        })
    }
}
