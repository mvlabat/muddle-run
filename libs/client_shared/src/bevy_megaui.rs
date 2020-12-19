use crate::{texture_node::MegaUiTextureNode, transform_node::MegaUiTransformNode};
use bevy::{
    app::{stage, AppBuilder, EventReader, Events, Plugin},
    asset::{AddAsset, Assets, Handle, HandleUntyped},
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
            RenderResourceBindings, RenderResourceContext, RenderResourceType,
        },
        shader::{Shader, ShaderStage, ShaderStages},
        texture::{Extent3d, Texture, TextureDimension, TextureFormat},
    },
    window::{CursorMoved, ReceivedCharacter, WindowResized, Windows},
};
use std::{borrow::Cow, collections::HashMap};

pub const MEGAUI_PIPELINE_HANDLE: HandleUntyped =
    HandleUntyped::weak_from_u64(PipelineDescriptor::TYPE_UUID, 9404026720151354217);
pub const MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME: &str = "MegaUiTransform";
pub const MEGAUI_TEXTURE_RESOURCE_BINDING_NAME: &str = "MegaUiTexture_texture";
pub const MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME: &str = "MegaUiTexture_texture_sampler";

pub struct MegaUiPlugin;

pub struct MegaUiContext {
    pub ui: megaui::Ui,
    ui_draw_lists: Vec<megaui::DrawList>,
    font_texture: Handle<MegaUiTexture>,
    megaui_textures: HashMap<u32, Handle<MegaUiTexture>>,

    mouse_position: (f32, f32),
    cursor: EventReader<CursorMoved>,
    received_character: EventReader<ReceivedCharacter>,
    resize: EventReader<WindowResized>,
}

impl MegaUiContext {
    pub fn new(ui: megaui::Ui, font_texture: Handle<MegaUiTexture>) -> Self {
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
    pub const MEGAUI_TEXTURE: &str = "megaui_texture";
    pub const MEGAUI_TRANSFORM: &str = "megaui_transform";
}

impl Plugin for MegaUiPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.add_system_to_stage(stage::PRE_UPDATE, process_input)
            .add_asset::<MegaUiTexture>();

        let resources = app.resources_mut();

        let ui = megaui::Ui::new();
        let font_texture = {
            let mut assets = resources.get_mut::<Assets<MegaUiTexture>>().unwrap();
            assets.add(MegaUiTexture {
                texture: Texture::new(
                    Extent3d::new(ui.font_atlas.texture.width, ui.font_atlas.texture.height, 1),
                    TextureDimension::D2,
                    ui.font_atlas.texture.data.clone(),
                    TextureFormat::Rgba8Unorm,
                ),
            })
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

        // Textures.
        render_graph.add_node(node::MEGAUI_TEXTURE, MegaUiTextureNode::new(font_texture));
        render_graph
            .add_node_edge(node::MEGAUI_TEXTURE, node::MEGAUI_PASS)
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
    texture_bind_group_descriptor: BindGroupDescriptor,
    texture_bind_group_id: Option<BindGroupId>,
    vertex_buffer: Option<BufferId>,
    index_buffer: Option<BufferId>,
}

impl MegaUiNode {
    pub fn new(
        pipeline_descriptor: Handle<PipelineDescriptor>,
        transform_bind_group_descriptor: BindGroupDescriptor,
        texture_bind_group_descriptor: BindGroupDescriptor,
        msaa: &Msaa,
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
            texture_bind_group_descriptor,
            texture_bind_group_id: None,
            vertex_buffer: None,
            index_buffer: None,
            color_resolve_target_indices,
        }
    }
}

struct DrawCommand {
    vertices_count: usize,
    #[allow(dead_code)]
    texture_handle: Handle<MegaUiTexture>,
    #[allow(dead_code)]
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
        let window_size = resources.get::<WindowSize>().unwrap();

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

        let render_resources = render_context.resources_mut();
        let render_resource_bindings = resources.get_mut::<RenderResourceBindings>().unwrap();

        if let Some(vertex_buffer_id) = self.vertex_buffer.take() {
            render_resources.remove_buffer(vertex_buffer_id);
        }
        if let Some(index_buffer_id) = self.index_buffer.take() {
            render_resources.remove_buffer(index_buffer_id);
        }

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

        if self.texture_bind_group_id.is_none() {
            let texture_bindings = render_resource_bindings
                .get(MEGAUI_TEXTURE_RESOURCE_BINDING_NAME)
                .unwrap()
                .clone();
            let texture_sampler_bindings = render_resource_bindings
                .get(MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME)
                .unwrap()
                .clone();
            let texture_bind_group = BindGroup::build()
                .add_binding(0, texture_bindings)
                .add_binding(1, texture_sampler_bindings)
                .finish();
            render_context
                .resources()
                .create_bind_group(self.texture_bind_group_descriptor.id, &texture_bind_group);
            self.texture_bind_group_id = Some(texture_bind_group.id);
        }

        let mut ctx = resources.get_thread_local_mut::<MegaUiContext>().unwrap();
        ctx.render_draw_lists();
        let mut ui_draw_lists = Vec::new();

        std::mem::swap(&mut ui_draw_lists, &mut ctx.ui_draw_lists);

        let mut vertex_buffer = Vec::<u8>::new();
        let mut index_buffer = Vec::new();
        let mut draw_commands = Vec::new();
        let mut index_offset = 0;

        // log::info!("FRAME");
        for draw_list in &ui_draw_lists {
            let texture_handle = if let Some(texture) = draw_list.texture {
                ctx.megaui_textures.get(&texture).unwrap().clone()
            } else {
                ctx.font_texture.clone()
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

            // for index in &draw_list.indices {
            //     let vertex = draw_list.vertices[*index as usize];
            //     println!(
            //         "idx: {}, pos: [{}, {}, {}]",
            //         *index, vertex.pos[0], vertex.pos[1], vertex.pos[2]
            //     );
            // }

            draw_commands.push(DrawCommand {
                vertices_count: draw_list.indices.len(),
                texture_handle,
                clipping_zone: draw_list.clipping_zone,
            });
        }
        self.vertex_buffer = Some(render_context.resources().create_buffer_with_data(
            BufferInfo {
                buffer_usage: BufferUsage::VERTEX,
                ..Default::default()
            },
            &vertex_buffer,
        ));
        self.index_buffer = Some(render_context.resources().create_buffer_with_data(
            BufferInfo {
                buffer_usage: BufferUsage::INDEX,
                ..Default::default()
            },
            &index_buffer,
        ));

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
                let mut vertex_offset: u32 = 0;
                for draw_command in &draw_commands {
                    if draw_command.texture_handle != ctx.font_texture {
                        panic!("Textures other than the font atlas are not supported yet");
                    }
                    render_pass.set_bind_group(
                        1,
                        self.texture_bind_group_descriptor.id,
                        self.texture_bind_group_id.unwrap(),
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
        std::mem::swap(&mut ui_draw_lists, &mut ctx.ui_draw_lists);
        ctx.ui
            .new_frame(resources.get::<Time>().unwrap().delta_seconds());
    }
}

#[derive(Debug, TypeUuid)]
#[uuid = "03b67fa3-bae5-4da3-8ffd-a1d696d9caf2"]
pub struct MegaUiTexture {
    pub texture: Texture,
}

// Is a thread local system, because `megaui::Ui` (`MegaUiContext`) doesn't implement Send + Sync.
fn process_input(
    _world: &mut World,
    resources: &mut Resources,
    // ctx: Local<MegaUiContext>,
    // ev_cursor: Res<Events<CursorMoved>>,
    // ev_keys: Res<Events<KeyboardInput>>,
    // mouse_button_input: Res<Input<MouseButton>>,
    // keyboard_input: Res<Input<KeyCode>>,
) {
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
