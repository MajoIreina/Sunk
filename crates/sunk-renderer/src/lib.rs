//! Minimal procedural black-hole renderer used by the Phase 0 vertical slice.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sunk_core::RenderQuality;
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

const SHADER_SOURCE: &str = include_str!("../../../shaders/blackhole.wgsl");
const VERTEX_ENTRY_POINT: &str = "vs_main";
const FRAGMENT_ENTRY_POINT: &str = "fs_main";
const PARAMETER_BIND_GROUP: u32 = 0;
const PARAMETER_BINDING: u32 = 0;

#[derive(Debug, Error)]
pub enum RendererError {
    #[error("failed to create the window surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no compatible GPU adapter was found: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create a GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("the selected adapter cannot present to this window")]
    UnsupportedSurface,
    #[error("the window surface does not support transparent composition; modes: {0:?}")]
    TransparencyUnsupported(Vec<wgpu::CompositeAlphaMode>),
    #[error("the GPU rejected the current surface frame")]
    SurfaceValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Presented,
    Skipped,
    Reconfigured,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ShaderParameters {
    resolution: [f32; 2],
    elapsed_seconds: f32,
    interaction: f32,
    ray_steps: u32,
    premultiply_output: u32,
    _padding: [u32; 2],
}

pub struct BlackHoleRenderer {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    parameter_buffer: wgpu::Buffer,
    parameter_bind_group: wgpu::BindGroup,
    size: PhysicalSize<u32>,
    quality: RenderQuality,
}

impl BlackHoleRenderer {
    /// Creates a transparent GPU surface and the Phase 0 render pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error when no compatible GPU or transparent composition mode is available.
    #[allow(
        clippy::too_many_lines,
        reason = "GPU initialization keeps the dependent surface and pipeline descriptors together"
    )]
    pub async fn new(window: Arc<Window>, quality: RenderQuality) -> Result<Self, RendererError> {
        let size = window.inner_size();
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        #[cfg(target_os = "windows")]
        {
            instance_descriptor.backend_options.dx12.presentation_system =
                wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        let instance = wgpu::Instance::new(instance_descriptor);
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Sunk GPU device"),
                ..Default::default()
            })
            .await?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or(RendererError::UnsupportedSurface)?;
        let alpha_mode = select_transparent_alpha_mode(&capabilities.alpha_modes)
            .ok_or_else(|| RendererError::TransparencyUnsupported(capabilities.alpha_modes))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(RendererError::UnsupportedSurface)?;
        config.format = format;
        config.alpha_mode = alpha_mode;
        surface.configure(&device, &config);

        let parameters = ShaderParameters {
            resolution: shader_resolution(size.width.max(1), size.height.max(1)),
            elapsed_seconds: 0.0,
            interaction: 0.0,
            ray_steps: quality.ray_steps,
            premultiply_output: u32::from(alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied),
            _padding: [0; 2],
        };
        let parameter_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sunk black-hole shader parameters"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sunk black-hole parameter layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: PARAMETER_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<ShaderParameters>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let parameter_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sunk black-hole parameters"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: PARAMETER_BINDING,
                resource: parameter_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sunk black-hole pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Procedural black-hole shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Sunk black-hole pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(VERTEX_ENTRY_POINT),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(FRAGMENT_ENTRY_POINT),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            parameter_buffer,
            parameter_bind_group,
            size,
            quality,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            self.size = size;
            return;
        }
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_quality(&mut self, quality: RenderQuality) {
        self.quality = quality;
    }

    /// Draws one frame of the procedural black hole.
    ///
    /// # Errors
    ///
    /// Returns an error if surface acquisition reports a GPU validation failure.
    pub fn render(
        &mut self,
        elapsed_seconds: f32,
        interaction: f32,
    ) -> Result<FrameOutcome, RendererError> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(FrameOutcome::Skipped);
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(FrameOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(FrameOutcome::Reconfigured);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RendererError::SurfaceValidation);
            }
        };

        let parameters = ShaderParameters {
            resolution: shader_resolution(self.size.width, self.size.height),
            elapsed_seconds,
            interaction: interaction.clamp(0.0, 1.0),
            ray_steps: self.quality.ray_steps,
            premultiply_output: u32::from(
                self.config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied,
            ),
            _padding: [0; 2],
        };
        self.queue
            .write_buffer(&self.parameter_buffer, 0, bytemuck::bytes_of(&parameters));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Sunk frame encoder"),
            });
        {
            let color_attachment = wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Sunk black-hole render pass"),
                color_attachments: &[Some(color_attachment)],
                ..Default::default()
            });
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(PARAMETER_BIND_GROUP, &self.parameter_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(FrameOutcome::Presented)
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "physical window dimensions are far below f32's exact integer range"
)]
fn shader_resolution(width: u32, height: u32) -> [f32; 2] {
    [width as f32, height as f32]
}

fn select_transparent_alpha_mode(
    modes: &[wgpu::CompositeAlphaMode],
) -> Option<wgpu::CompositeAlphaMode> {
    #[cfg(target_os = "windows")]
    let preferred = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
    ];
    #[cfg(not(target_os = "windows"))]
    let preferred = [
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::PreMultiplied,
    ];

    preferred.into_iter().find(|mode| modes.contains(mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::{
        AddressSpace, ScalarKind, ShaderStage, TypeInner, VectorSize,
        valid::{Capabilities, ValidationFlags, Validator},
    };

    fn parse_and_validate_shader() -> wgpu::naga::Module {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER_SOURCE)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(SHADER_SOURCE)));
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(SHADER_SOURCE)));
        module
    }

    #[test]
    fn wgsl_parses_and_validates() {
        parse_and_validate_shader();
    }

    #[test]
    fn shader_entry_points_match_pipeline_contract() {
        let module = parse_and_validate_shader();
        let entry_points = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect::<Vec<_>>();

        assert_eq!(entry_points.len(), 2);
        assert!(entry_points.contains(&(VERTEX_ENTRY_POINT, ShaderStage::Vertex)));
        assert!(entry_points.contains(&(FRAGMENT_ENTRY_POINT, ShaderStage::Fragment)));
    }

    #[test]
    fn shader_uniform_layout_matches_host() {
        let module = parse_and_validate_shader();
        let (_, parameters) = module
            .global_variables
            .iter()
            .find(|(_, variable)| variable.name.as_deref() == Some("params"))
            .expect("shader must expose the params uniform");

        assert_eq!(parameters.space, AddressSpace::Uniform);
        let binding = parameters
            .binding
            .expect("params must have a resource binding");
        assert_eq!(
            (binding.group, binding.binding),
            (PARAMETER_BIND_GROUP, PARAMETER_BINDING)
        );
        assert_eq!(
            module
                .global_variables
                .iter()
                .filter(|(_, variable)| variable.binding.is_some())
                .count(),
            1,
            "the production pipeline currently binds exactly one shader resource"
        );

        let TypeInner::Struct { members, span } = &module.types[parameters.ty].inner else {
            panic!("params must use the Parameters structure");
        };
        assert_eq!(*span, 32);

        let expected_members = [
            ("resolution", 0, Some(VectorSize::Bi), ScalarKind::Float),
            ("elapsed_seconds", 8, None, ScalarKind::Float),
            ("interaction", 12, None, ScalarKind::Float),
            ("ray_steps", 16, None, ScalarKind::Uint),
            ("premultiply_output", 20, None, ScalarKind::Uint),
            ("_padding", 24, Some(VectorSize::Bi), ScalarKind::Uint),
        ];
        assert_eq!(members.len(), expected_members.len());

        for (member, (name, offset, vector_size, scalar_kind)) in
            members.iter().zip(expected_members)
        {
            assert_eq!(member.name.as_deref(), Some(name));
            assert_eq!(member.offset, offset);
            match (&module.types[member.ty].inner, vector_size) {
                (TypeInner::Scalar(scalar), None) => {
                    assert_eq!(scalar.kind, scalar_kind);
                    assert_eq!(scalar.width, 4);
                }
                (TypeInner::Vector { size, scalar }, Some(expected_size)) => {
                    assert_eq!(*size, expected_size);
                    assert_eq!(scalar.kind, scalar_kind);
                    assert_eq!(scalar.width, 4);
                }
                (actual, expected) => {
                    panic!("unexpected type for {name}: {actual:?}; vector size {expected:?}")
                }
            }
        }

        assert_eq!(std::mem::size_of::<ShaderParameters>(), 32);
        assert_eq!(std::mem::offset_of!(ShaderParameters, resolution), 0);
        assert_eq!(std::mem::offset_of!(ShaderParameters, elapsed_seconds), 8);
        assert_eq!(std::mem::offset_of!(ShaderParameters, interaction), 12);
        assert_eq!(std::mem::offset_of!(ShaderParameters, ray_steps), 16);
        assert_eq!(
            std::mem::offset_of!(ShaderParameters, premultiply_output),
            20
        );
        assert_eq!(std::mem::offset_of!(ShaderParameters, _padding), 24);
    }

    #[test]
    fn transparent_alpha_mode_uses_the_platform_preference() {
        let modes = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ];
        let expected = if cfg!(target_os = "windows") {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            wgpu::CompositeAlphaMode::PostMultiplied
        };
        assert_eq!(select_transparent_alpha_mode(&modes), Some(expected));
    }

    #[test]
    fn a_single_transparent_mode_is_accepted() {
        assert_eq!(
            select_transparent_alpha_mode(&[wgpu::CompositeAlphaMode::PreMultiplied]),
            Some(wgpu::CompositeAlphaMode::PreMultiplied)
        );
    }

    #[test]
    fn opaque_only_surface_is_rejected() {
        assert_eq!(
            select_transparent_alpha_mode(&[wgpu::CompositeAlphaMode::Opaque]),
            None
        );
    }
}
