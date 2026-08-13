using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.RenderGraphModule;
using UnityEngine.Rendering.Universal;

namespace Sunk.Rendering.Gargantua
{
    public sealed class GargantuaRendererFeature : ScriptableRendererFeature
    {
        private const string ShaderName = "Hidden/Sunk/Gargantua";

        [SerializeField] private GargantuaSettings settings;
        [SerializeField] private Shader shader;
        [SerializeField] private RenderPassEvent injectionPoint = RenderPassEvent.BeforeRenderingPostProcessing;

        private Material material;
        private GargantuaRenderPass renderPass;

        public GargantuaSettings Settings => settings;

        public override void Create()
        {
            DisposeMaterial();
            shader = shader != null ? shader : Shader.Find(ShaderName);
            material = shader != null ? CoreUtils.CreateEngineMaterial(shader) : null;
            renderPass = new GargantuaRenderPass
            {
                renderPassEvent = injectionPoint
            };
        }

        public override void AddRenderPasses(ScriptableRenderer renderer, ref RenderingData renderingData)
        {
            CameraType cameraType = renderingData.cameraData.cameraType;
            if (settings == null || material == null ||
                (cameraType != CameraType.Game && cameraType != CameraType.SceneView))
            {
                return;
            }

            GargantuaShaderProperties.Apply(material, settings);
            renderPass.Setup(material);
            renderer.EnqueuePass(renderPass);
        }

        public bool Configure(GargantuaSettings newSettings, Shader newShader = null)
        {
            Shader resolvedShader = newShader != null ? newShader : Shader.Find(ShaderName);
            bool changed = settings != newSettings || shader != resolvedShader;
            if (changed)
            {
                settings = newSettings;
                shader = resolvedShader;
                Create();
            }

            return changed;
        }

        protected override void Dispose(bool disposing)
        {
            DisposeMaterial();
            renderPass = null;
        }

        private void DisposeMaterial()
        {
            CoreUtils.Destroy(material);
            material = null;
        }

        private sealed class GargantuaRenderPass : ScriptableRenderPass
        {
            private const string PassName = "Sunk Gargantua";
            private static readonly ProfilingSampler ProfilingSampler = new(PassName);

            private Material material;

            public void Setup(Material newMaterial)
            {
                material = newMaterial;
            }

            public override void RecordRenderGraph(RenderGraph renderGraph, ContextContainer frameData)
            {
                UniversalResourceData resourceData = frameData.Get<UniversalResourceData>();
                UniversalCameraData cameraData = frameData.Get<UniversalCameraData>();
                CameraType cameraType = cameraData.camera.cameraType;

                if (material == null ||
                    (cameraType != CameraType.Game && cameraType != CameraType.SceneView) ||
                    !resourceData.activeColorTexture.IsValid())
                {
                    return;
                }

                using IRasterRenderGraphBuilder builder =
                    renderGraph.AddRasterRenderPass<PassData>(PassName, out PassData passData, ProfilingSampler);
                passData.material = material;
                builder.SetRenderAttachment(resourceData.activeColorTexture, 0, AccessFlags.WriteAll);
                builder.SetRenderFunc(static (PassData data, RasterGraphContext context) =>
                {
                    context.cmd.DrawProcedural(
                        Matrix4x4.identity,
                        data.material,
                        0,
                        MeshTopology.Triangles,
                        3,
                        1);
                });
            }

#pragma warning disable CS0672, CS0618
            public override void Execute(ScriptableRenderContext context, ref RenderingData renderingData)
            {
                if (material == null)
                {
                    return;
                }

                CommandBuffer commandBuffer = CommandBufferPool.Get(PassName);
                using (new ProfilingScope(commandBuffer, ProfilingSampler))
                {
                    RTHandle target = renderingData.cameraData.renderer.cameraColorTargetHandle;
                    CoreUtils.SetRenderTarget(commandBuffer, target, ClearFlag.None, Color.clear);
                    commandBuffer.DrawProcedural(
                        Matrix4x4.identity,
                        material,
                        0,
                        MeshTopology.Triangles,
                        3,
                        1);
                }

                context.ExecuteCommandBuffer(commandBuffer);
                CommandBufferPool.Release(commandBuffer);
            }
#pragma warning restore CS0672, CS0618

            private sealed class PassData
            {
                public Material material;
            }
        }
    }
}
