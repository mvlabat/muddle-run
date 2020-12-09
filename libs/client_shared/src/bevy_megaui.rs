use crate::transform_node::MegaUiTransformNode;
use bevy::{
    app::{stage, AppBuilder, EventReader, Events, Plugin},
    asset::{AddAsset, Assets, Handle, HandleUntyped},
    core::{AsBytes, Byteable},
    ecs::{Bundle, Commands, IntoSystem, Local, Res, ResMut, Resources, System, World},
    input::{
        keyboard::{KeyCode, KeyboardInput},
        mouse::{MouseButton, MouseButtonInput},
        Input,
    },
    math::{Rect, Vec2, Vec3},
    reflect::{Reflect, ReflectComponent, TypeUuid},
    render::{
        camera::{Camera, OrthographicProjection, VisibleEntities, WindowOrigin},
        color::Color,
        mesh::VertexAttributeValues,
        pass::{
            LoadOp, Operations, PassDescriptor, RenderPassDepthStencilAttachmentDescriptor,
            TextureAttachment,
        },
        pipeline::{
            AsVertexFormats, BindGroupDescriptor, BindType, BindingDescriptor, BindingShaderStage,
            BlendDescriptor, BlendFactor, BlendOperation, ColorStateDescriptor, ColorWrite,
            CompareFunction, CullMode, DepthStencilStateDescriptor, FrontFace, IndexFormat,
            InputStepMode, PipelineCompiler, PipelineDescriptor, PipelineSpecialization,
            RasterizationStateDescriptor, StencilStateDescriptor, StencilStateFaceDescriptor,
            UniformProperty, VertexAttributeDescriptor, VertexBufferDescriptor, VertexFormat,
        },
        render_graph::{
            base, base::Msaa, AssetRenderResourcesNode, CommandQueue, Node, PassNode, RenderGraph,
            ResourceSlotInfo, ResourceSlots, SystemNode, WindowSwapChainNode, WindowTextureNode,
        },
        renderer::{
            BindGroup, BindGroupId, BufferId, RenderContext, RenderResource,
            RenderResourceBindings, RenderResourceContext, RenderResourceType, RenderResources,
        },
        shader::{Shader, ShaderStage, ShaderStages},
        texture::{Extent3d, Texture, TextureDimension, TextureFormat},
    },
    transform::components::{GlobalTransform, Transform},
    window::{CursorMoved, WindowDescriptor, WindowResized},
};
use megaui::Vertex;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

pub const MEGAUI_PIPELINE_HANDLE: HandleUntyped =
    HandleUntyped::weak_from_u64(PipelineDescriptor::TYPE_UUID, 9404026720151354217);
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

#[derive(Debug, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct DrawMegaUi {}

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

pub mod camera {
    pub const MEGA_UI_CAMERA: &str = "MegaUiCamera";
}

impl Plugin for MegaUiPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.add_system_to_stage(stage::POST_UPDATE, process_input)
            .add_asset::<MegaUiTexture>();

        let resources = app.resources_mut();

        let ui = megaui::Ui::new();
        let font_texture = {
            let mut assets = resources.get_mut::<Assets<Texture>>().unwrap();
            assets.add(Texture::new(
                Extent3d::new(ui.font_atlas.texture.width, ui.font_atlas.texture.height, 1),
                TextureDimension::D2,
                ui.font_atlas.texture.data.clone(),
                TextureFormat::Rgba8UnormSrgb,
            ))
        };
        resources.insert(WindowSize::new(0.0, 0.0));
        resources.insert_thread_local(MegaUiContext::new(ui, font_texture));

        let msaa = resources.get::<Msaa>().unwrap();
        let mut shaders = resources.get_mut::<Assets<Shader>>().unwrap();

        let mut pipelines = resources.get_mut::<Assets<PipelineDescriptor>>().unwrap();
        pipelines.set_untracked(MEGAUI_PIPELINE_HANDLE, build_megaui_pipeline(&mut shaders));

        let mut render_graph = resources.get_mut::<RenderGraph>().unwrap();

        let mut megaui_pass_node = PassNode::<&DrawMegaUi>::new(PassDescriptor {
            color_attachments: vec![msaa.color_attachment_descriptor(
                TextureAttachment::Input("color_attachment".to_string()),
                TextureAttachment::Input("color_resolve_target".to_string()),
                Operations {
                    load: LoadOp::Load,
                    store: true,
                },
            )],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachmentDescriptor {
                attachment: TextureAttachment::Input("depth".to_string()),
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: true,
                }),
                stencil_ops: None,
            }),
            sample_count: msaa.samples,
        });

        megaui_pass_node.add_camera(camera::MEGA_UI_CAMERA);
        render_graph.add_node(node::MEGAUI_PASS, megaui_pass_node);

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

        // Ensure megaui pass runs after main pass.
        render_graph.add_node_edge(base::node::MAIN_PASS, node::MEGAUI_PASS)
            .unwrap();
        // render_graph.add_node_edge(bevy::ui::render::node::UI_PASS, node::MEGAUI_PASS)
        //     .unwrap();

        // Transform.
        // render_graph.add_system_node(node::MEGAUI_TRANSFORM, MegaUiTransformNode::new());
        // render_graph
        //     .add_node_edge(node::MEGAUI_TRANSFORM, node::MEGAUI_PASS)
        //     .unwrap();

        // Textures.
        // render_graph.add_system_node(
        //     node::MEGAUI_ASSET,
        //     AssetRenderResourcesNode::<MegaUiTexture>::new(false),
        // );
        // render_graph
        //     .add_node_edge(node::MEGAUI_ASSET, node::MEGAUI_PASS)
        //     .unwrap();
    }
}

pub struct MegaUiNodeState {
    clipping_zone: Option<megaui::Rect>,
}

#[derive(Bundle, Debug)]
pub struct MegaUiCameraBundle {
    pub camera: Camera,
    pub orthographic_projection: OrthographicProjection,
    pub visible_entities: VisibleEntities,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
}

impl Default for MegaUiCameraBundle {
    fn default() -> Self {
        let far = 1000.0;
        Self {
            camera: Camera {
                name: Some(camera::MEGA_UI_CAMERA.to_string()),
                ..Default::default()
            },
            orthographic_projection: OrthographicProjection {
                far,
                window_origin: WindowOrigin::BottomLeft,
                ..Default::default()
            },
            visible_entities: Default::default(),
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, far - 0.1)),
            global_transform: Default::default(),
        }
    }
}

pub struct MegaUiDrawable;

struct DrawCommand {
    vertices_count: usize,
    texture_handle: Handle<Texture>,
    clipping_zone: Option<megaui::Rect>,
}

// impl Node for MegaUiNode {
//     fn input(&self) -> &[ResourceSlotInfo] {
//         &self.inputs
//     }
//
//     fn update(
//         &mut self,
//         _world: &World,
//         resources: &Resources,
//         render_context: &mut dyn RenderContext,
//         _input: &ResourceSlots,
//         _output: &mut ResourceSlots,
//     ) {
//         let mut ctx = resources.get_thread_local_mut::<MegaUiContext>().unwrap();
//         ctx.render_draw_lists();
//         let mut ui_draw_lists = Vec::new();
//
//         std::mem::swap(&mut ui_draw_lists, &mut ctx.ui_draw_lists);
//
//         let mut vertex_buffer = Vec::<u8>::new();
//         let mut index_buffer = Vec::new();
//         let mut draw_commands = Vec::new();
//
//         for draw_list in &ui_draw_lists {
//             let texture_handle = if let Some(texture) = draw_list.texture {
//                 ctx.megaui_textures.get(&texture).unwrap().clone()
//             } else {
//                 ctx.font_texture.clone()
//             };
//
//             for vertex in &draw_list.vertices {
//                 vertex_buffer.extend_from_slice(vertex.pos.as_bytes());
//                 vertex_buffer.extend_from_slice(vertex.uv.as_bytes());
//                 vertex_buffer.extend_from_slice(vertex.color.as_bytes());
//             }
//             index_buffer.extend_from_slice(draw_list.indices.as_slice().as_bytes());
//
//             draw_commands.push(DrawCommand {
//                 vertices_count: draw_list.indices.len(),
//                 texture_handle,
//                 clipping_zone: draw_list.clipping_zone,
//             });
//         }
//     }
// }
//
// impl SystemNode for MegaUiNode {
//     fn get_system(&self, commands: &mut Commands) -> Box<dyn System<Input = (), Output = ()>> {
//         let system = render_megaui_system.system();
//         commands.insert_local_resource(
//             system.id(),
//             MegaUiNodeState {
//                 // command_queue: self.command_queue.clone(),
//             },
//         );
//         Box::new(system)
//     }
// }

fn render_megaui_system(_world: &mut World, resources: &mut Resources) {}

#[derive(Debug, RenderResources, TypeUuid)]
#[uuid = "03b67fa3-bae5-4da3-8ffd-a1d696d9caf2"]
pub struct MegaUiTexture {
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
            front_face: FrontFace::Ccw,
            cull_mode: CullMode::Back,
            depth_bias: 0,
            depth_bias_slope_scale: 0.0,
            depth_bias_clamp: 0.0,
            clamp_depth: false,
        }),
        depth_stencil_state: Some(DepthStencilStateDescriptor {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: CompareFunction::Less,
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
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            write_mask: ColorWrite::ALL,
        }],
        // index_format: IndexFormat::Uint16,
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
