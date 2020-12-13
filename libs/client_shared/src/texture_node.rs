use crate::bevy_megaui::{
    MegaUiTexture, MEGAUI_TEXTURE_RESOURCE_BINDING_NAME,
    MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME,
};
use bevy::{
    asset::{Assets, Handle},
    ecs::{Resources, World},
    render::{
        render_graph::{Node, ResourceSlots},
        renderer::{
            BufferInfo, BufferUsage, RenderContext, RenderResourceBinding, RenderResourceBindings,
            RenderResourceId,
        },
        texture,
        texture::TextureDescriptor,
    },
};

#[derive(Debug)]
pub struct MegaUiTextureNode {
    font_texture_handle: Handle<MegaUiTexture>,
    initialized: bool,
}

impl MegaUiTextureNode {
    pub fn new(font_texture_handle: Handle<MegaUiTexture>) -> Self {
        MegaUiTextureNode {
            font_texture_handle,
            initialized: false,
        }
    }
}

impl Node for MegaUiTextureNode {
    fn update(
        &mut self,
        _world: &World,
        resources: &Resources,
        render_context: &mut dyn RenderContext,
        _input: &ResourceSlots,
        _output: &mut ResourceSlots,
    ) {
        let mut render_resource_bindings = resources.get_mut::<RenderResourceBindings>().unwrap();
        let megaui_texture_assets = resources.get_mut::<Assets<MegaUiTexture>>().unwrap();

        if !self.initialized {
            let render_resource_context = render_context.resources();

            let font_texture = megaui_texture_assets
                .get(self.font_texture_handle.clone())
                .unwrap();
            let aligned_width = render_context
                .resources()
                .get_aligned_texture_size(font_texture.texture.size.width as usize);

            let texture_descriptor: TextureDescriptor = (&font_texture.texture).into();
            let texture_resource = render_resource_context.create_texture(texture_descriptor);
            let sampler_resource =
                render_resource_context.create_sampler(&font_texture.texture.sampler);

            render_resource_context.set_asset_resource(
                &self.font_texture_handle,
                RenderResourceId::Texture(texture_resource),
                texture::TEXTURE_ASSET_INDEX,
            );
            render_resource_context.set_asset_resource(
                &self.font_texture_handle,
                RenderResourceId::Sampler(sampler_resource),
                texture::SAMPLER_ASSET_INDEX,
            );

            render_resource_bindings.set(
                MEGAUI_TEXTURE_RESOURCE_BINDING_NAME,
                RenderResourceBinding::Texture(texture_resource),
            );
            render_resource_bindings.set(
                MEGAUI_TEXTURE_SAMPLER_RESOURCE_BINDING_NAME,
                RenderResourceBinding::Sampler(sampler_resource),
            );

            let format_size = font_texture.texture.format.pixel_size();

            let texture_buffer = render_context.resources().create_buffer_with_data(
                BufferInfo {
                    buffer_usage: BufferUsage::COPY_SRC,
                    ..Default::default()
                },
                &font_texture.texture.data,
            );

            render_context.copy_buffer_to_texture(
                texture_buffer,
                0,
                (format_size * aligned_width) as u32,
                texture_resource,
                [0, 0, 0],
                0,
                texture_descriptor.size,
            );
            render_context.resources().remove_buffer(texture_buffer);

            self.initialized = true;
        }
    }
}
