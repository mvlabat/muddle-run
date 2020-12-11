use crate::transform_node::MegaUiTransformNode;
use bevy::{
    app::{stage, AppBuilder, EventReader, Events, Plugin},
    asset::{AddAsset, Assets, Handle},
    core::{AsBytes},
    ecs::{Commands, IntoSystem, Resources, System, World},
    input::{
        keyboard::{KeyCode, KeyboardInput},
        mouse::{MouseButton},
        Input,
    },
    math::{Vec2},
    render::{
        pass::{
            ClearColor, LoadOp, Operations, PassDescriptor,
            RenderPassDepthStencilAttachmentDescriptor, TextureAttachment,
        },
        pipeline::{
            BindGroupDescriptor, BindType, BindingDescriptor, BindingShaderStage,
            BlendDescriptor, BlendFactor, BlendOperation, ColorStateDescriptor, ColorWrite,
            CompareFunction, CullMode, DepthStencilStateDescriptor, FrontFace, IndexFormat,
            InputStepMode, PipelineCompiler, PipelineDescriptor, PipelineSpecialization,
            RasterizationStateDescriptor, StencilStateDescriptor, StencilStateFaceDescriptor,
            UniformProperty, VertexAttributeDescriptor, VertexBufferDescriptor, VertexFormat,
        },
        render_graph::{
            base, base::Msaa, CommandQueue, Node, RenderGraph,
            ResourceSlotInfo, ResourceSlots, SystemNode, WindowSwapChainNode, WindowTextureNode,
        },
        renderer::{
            BindGroup, BindGroupId, BufferId, BufferInfo, BufferUsage, RenderContext,
            RenderResourceBindings, RenderResourceType,
            RenderResources,
        },
        shader::{Shader, ShaderStage, ShaderStages},
        texture::{Texture, TextureFormat},
    },
    type_registry::TypeUuid,
    window::{CursorMoved, WindowDescriptor, WindowResized},
};
use std::{borrow::Cow, collections::HashMap};
use bevy::core::Time;

pub const MEGAUI_PIPELINE_HANDLE: Handle<PipelineDescriptor> =
    Handle::weak_from_u64(PipelineDescriptor::TYPE_UUID, 9404026720151354217);
pub const MEGAUI_TRANSFORM_RESOURCE_BINDING_NAME: &str = "MegaUiTransform";

pub struct MegaUiPlugin;

pub struct MegaUiContext {
    pub ui: megaui::Ui,
    ui_draw_lists: Vec<megaui::DrawList>,
    font_texture: Handle<Texture>,
    megaui_textures: HashMap<u32, Handle<Texture>>,

    mouse_position: (f32, f32),
    cursor: EventReader<CursorMoved>,
    keys: EventReader<KeyboardInput>,
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
            keys: Default::default(),
            resize: Default::default(),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl WindowSize {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
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
    pub const MEGAUI_ASSET: &str = "megaui_asset";
    pub const MEGAUI_TRANSFORM: &str = "megaui_transform";
}

impl Plugin for MegaUiPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.add_system_to_stage(stage::POST_UPDATE, process_input)
            .add_asset::<MegaUiAsset>();

        let resources = app.resources_mut();

        let ui = megaui::Ui::new();
        let font_texture = {
            let mut assets = resources.get_mut::<Assets<Texture>>().unwrap();
            assets.add(Texture::new(
                Vec2::new(
                    ui.font_atlas.texture.width as f32,
                    ui.font_atlas.texture.height as f32,
                ),
                ui.font_atlas.texture.data.clone(),
                TextureFormat::Rgba8UnormSrgb,
            ))
        };
        resources.insert(WindowSize::new(0.0, 0.0));
        resources.insert_thread_local(MegaUiContext::new(ui, font_texture));

        let msaa = resources.get::<Msaa>().unwrap();

        let mut render_graph = resources.get_mut::<RenderGraph>().unwrap();

        render_graph.add_node(node::MEGAUI_PASS, MegaUiNode::new(&msaa));
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
        // render_graph.add_system_node(
        //     node::MEGAUI_ASSET,
        //     AssetRenderResourcesNode::<MegaUiAsset>::new(false),
        // );
        // render_graph
        //     .add_node_edge(node::MEGAUI_ASSET, node::MEGAUI_PASS)
        //     .unwrap();

        let mut pipelines = resources.get_mut::<Assets<PipelineDescriptor>>().unwrap();
        let mut shaders = resources.get_mut::<Assets<Shader>>().unwrap();
        pipelines.set_untracked(MEGAUI_PIPELINE_HANDLE, build_megaui_pipeline(&mut shaders));
    }
}

pub struct MegaUiNode {
    pass_descriptor: PassDescriptor,
    inputs: Vec<ResourceSlotInfo>,
    color_attachment_input_indices: Vec<Option<usize>>,
    color_resolve_target_indices: Vec<Option<usize>>,
    depth_stencil_attachment_input_index: Option<usize>,
    default_clear_color_inputs: Vec<usize>,
    transform_bind_group_descriptor: BindGroupDescriptor,
    transform_bind_group_id: Option<BindGroupId>,
    command_queue: CommandQueue,
    vertex_buffer: Option<BufferId>,
    index_buffer: Option<BufferId>,
}

impl MegaUiNode {
    pub fn new(msaa: &Msaa) -> Self {
        let transform_bind_group_descriptor = BindGroupDescriptor::new(
            0,
            vec![BindingDescriptor {
                name: "MegaUiTransform".to_string(),
                index: 0,
                bind_type: BindType::Uniform {
                    dynamic: false,
                    property: UniformProperty::Struct(vec![
                        UniformProperty::Vec2,
                        UniformProperty::Vec2,
                    ]),
                },
                shader_stage: BindingShaderStage::VERTEX,
            }],
        );
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
            default_clear_color_inputs: Vec::new(),
            inputs,
            depth_stencil_attachment_input_index,
            color_attachment_input_indices,
            transform_bind_group_descriptor,
            transform_bind_group_id: None,
            command_queue: CommandQueue::default(),
            vertex_buffer: None,
            index_buffer: None,
            color_resolve_target_indices,
        }
    }
}

pub struct MegaUiNodeState {
    // command_queue: CommandQueue,
}

struct DrawCommand {
    vertices_count: usize,
    texture_handle: Handle<Texture>,
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
        let pipeline_descriptor = {
            let mut pipelines = resources.get_mut::<Assets<PipelineDescriptor>>().unwrap();
            let mut shaders = resources.get_mut::<Assets<Shader>>().unwrap();
            let render_resource_context = render_context.resources();
            let mut pipeline_compiler = resources.get_mut::<PipelineCompiler>().unwrap();

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
                    color_attachment.attachment = TextureAttachment::Id(
                        input.get(input_index).unwrap().get_texture().unwrap(),
                    );
                }
                if let Some(input_index) = self.color_resolve_target_indices[i] {
                    color_attachment.resolve_target = Some(TextureAttachment::Id(
                        input.get(input_index).unwrap().get_texture().unwrap(),
                    ));
                }
            }

            if let Some(input_index) = self.depth_stencil_attachment_input_index {
                self.pass_descriptor
                    .depth_stencil_attachment
                    .as_mut()
                    .unwrap()
                    .attachment =
                    TextureAttachment::Id(input.get(input_index).unwrap().get_texture().unwrap());
            }

            let attributes = vec![
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Position"),
                    offset: 0,
                    format: VertexFormat::Float3,
                    shader_location: 0,
                },
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Uv"),
                    offset: 0,
                    format: VertexFormat::Float2,
                    shader_location: 1,
                },
                VertexAttributeDescriptor {
                    name: Cow::from("Vertex_Color"),
                    offset: 0,
                    format: VertexFormat::Float4,
                    shader_location: 0,
                },
            ];
            pipeline_compiler.compile_pipeline(
                render_resource_context,
                &mut pipelines,
                &mut shaders,
                &MEGAUI_PIPELINE_HANDLE,
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

        let mut ctx = resources.get_thread_local_mut::<MegaUiContext>().unwrap();
        ctx.render_draw_lists();
        let mut ui_draw_lists = Vec::new();

        std::mem::swap(&mut ui_draw_lists, &mut ctx.ui_draw_lists);

        let mut vertex_buffer = Vec::<u8>::new();
        let mut index_buffer = Vec::new();
        let mut draw_commands = Vec::new();

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
            index_buffer.extend_from_slice(draw_list.indices.as_slice().as_bytes());

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
                render_pass.set_pipeline(&pipeline_descriptor);
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
                    // render_pass.set_scissor_rect()
                    // Megaui returns an empty DrawList as the last one for some reason.
                    if draw_command.vertices_count > 0 {
                        render_pass.draw_indexed(
                            vertex_offset..(vertex_offset + draw_command.vertices_count as u32),
                            0,
                            0..1,
                        );
                    }
                    vertex_offset += draw_command.vertices_count as u32;
                }
            },
        );
        ctx.ui.new_frame(resources.get::<Time>().unwrap().delta_seconds);
    }
}

impl SystemNode for MegaUiNode {
    fn get_system(&self, commands: &mut Commands) -> Box<dyn System<Input = (), Output = ()>> {
        let system = render_megaui_system.system();
        commands.insert_local_resource(
            system.id(),
            MegaUiNodeState {
                // command_queue: self.command_queue.clone(),
            },
        );
        Box::new(system)
    }
}

fn render_megaui_system(_world: &mut World, _resources: &mut Resources) {}

#[derive(Debug, RenderResources, TypeUuid)]
#[uuid = "03b67fa3-bae5-4da3-8ffd-a1d696d9caf2"]
pub struct MegaUiAsset {
    pub texture: Handle<Texture>,
}

// impl MegaUiTexture {
//     fn new(texture: Handle<Texture>) -> Self {
//         Self { texture }
//     }
// }

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
    let ev_keys = resources.get::<Events<KeyboardInput>>().unwrap();
    let ev_resize = resources.get::<Events<WindowResized>>().unwrap();
    let mouse_button_input = resources.get::<Input<MouseButton>>().unwrap();
    let keyboard_input = resources.get::<Input<KeyCode>>().unwrap();
    let window_descriptor = resources.get::<WindowDescriptor>().unwrap();
    let mut window_size = resources.get_mut::<WindowSize>().unwrap();

    if *window_size == WindowSize::new(0.0, 0.0) {
        *window_size = WindowSize::new(
            window_descriptor.width as f32,
            window_descriptor.height as f32,
        );
    }
    if let Some(resize_event) = ctx.resize.latest(&ev_resize) {
        *window_size = WindowSize::new(resize_event.width as f32, resize_event.height as f32);
    }

    if let Some(cursor_moved) = ctx.cursor.latest(&ev_cursor) {
        let mouse_position = cursor_moved.position.into();
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

    for keyboard_input in ctx.keys.iter(&ev_keys) {
        if let Some(pressed_char) = keyboard_input.key_code.and_then(keycode_to_char) {
            if !ctrl {
                ctx.ui.char_event(pressed_char, false, false);
            }
        }
    }

    if keyboard_input.just_released(KeyCode::Up) {
        ctx.ui.key_down(megaui::KeyCode::Up, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Down) {
        ctx.ui.key_down(megaui::KeyCode::Down, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Right) {
        ctx.ui.key_down(megaui::KeyCode::Right, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Left) {
        ctx.ui.key_down(megaui::KeyCode::Left, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Home) {
        ctx.ui.key_down(megaui::KeyCode::Home, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::End) {
        ctx.ui.key_down(megaui::KeyCode::End, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Delete) {
        ctx.ui.key_down(megaui::KeyCode::Delete, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Back) {
        ctx.ui.key_down(megaui::KeyCode::Backspace, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Return) {
        ctx.ui.key_down(megaui::KeyCode::Enter, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Tab) {
        ctx.ui.key_down(megaui::KeyCode::Tab, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Z) {
        ctx.ui.key_down(megaui::KeyCode::Z, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::Y) {
        ctx.ui.key_down(megaui::KeyCode::Y, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::C) {
        ctx.ui.key_down(megaui::KeyCode::C, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::X) {
        ctx.ui.key_down(megaui::KeyCode::X, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::V) {
        ctx.ui.key_down(megaui::KeyCode::V, shift, ctrl);
    }
    if keyboard_input.just_released(KeyCode::A) {
        ctx.ui.key_down(megaui::KeyCode::A, shift, ctrl);
    }
}

fn keycode_to_char(key_code: KeyCode) -> Option<char> {
    match key_code {
        KeyCode::A => Some('A'),
        KeyCode::B => Some('B'),
        KeyCode::C => Some('C'),
        KeyCode::D => Some('D'),
        KeyCode::E => Some('E'),
        KeyCode::F => Some('F'),
        KeyCode::G => Some('G'),
        KeyCode::H => Some('H'),
        KeyCode::I => Some('I'),
        KeyCode::J => Some('J'),
        KeyCode::K => Some('K'),
        KeyCode::L => Some('L'),
        KeyCode::M => Some('M'),
        KeyCode::N => Some('N'),
        KeyCode::O => Some('O'),
        KeyCode::P => Some('P'),
        KeyCode::Q => Some('Q'),
        KeyCode::R => Some('R'),
        KeyCode::S => Some('S'),
        KeyCode::T => Some('T'),
        KeyCode::U => Some('U'),
        KeyCode::V => Some('V'),
        KeyCode::W => Some('W'),
        KeyCode::X => Some('X'),
        KeyCode::Y => Some('Y'),
        KeyCode::Z => Some('Z'),
        _ => None,
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
