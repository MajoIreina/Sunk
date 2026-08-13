using UnityEngine;

namespace Sunk.Rendering.Gargantua
{
    internal static class GargantuaShaderProperties
    {
        private static readonly int Mass = Shader.PropertyToID("_Mass");
        private static readonly int Spin = Shader.PropertyToID("_Spin");
        private static readonly int HorizonRadius = Shader.PropertyToID("_HorizonRadius");
        private static readonly int ApparentShadowRadius = Shader.PropertyToID("_ApparentShadowRadius");
        private static readonly int ScreenShadowRadius = Shader.PropertyToID("_ScreenShadowRadius");
        private static readonly int LensingStrength = Shader.PropertyToID("_LensingStrength");
        private static readonly int DiskRadii = Shader.PropertyToID("_DiskRadii");
        private static readonly int DiskAppearance = Shader.PropertyToID("_DiskAppearance");
        private static readonly int Relativity = Shader.PropertyToID("_Relativity");
        private static readonly int SecondaryImage = Shader.PropertyToID("_SecondaryImage");
        private static readonly int Environment = Shader.PropertyToID("_Environment");

        public static void Apply(Material material, GargantuaSettings settings)
        {
            material.SetFloat(Mass, settings.Mass);
            material.SetFloat(Spin, settings.Spin);
            material.SetFloat(HorizonRadius, settings.HorizonRadius);
            material.SetFloat(ApparentShadowRadius, settings.ApparentShadowRadius);
            material.SetFloat(ScreenShadowRadius, settings.ScreenShadowRadius);
            material.SetFloat(LensingStrength, settings.LensingStrength);
            material.SetVector(DiskRadii, new Vector4(
                settings.DiskInnerRadius,
                settings.DiskOuterRadius,
                settings.DiskHalfThickness,
                settings.DiskTemperature));
            material.SetVector(DiskAppearance, new Vector4(
                settings.DiskEmission,
                settings.Turbulence,
                settings.RotationSpeed,
                settings.PhotonRingIntensity));
            material.SetVector(Relativity, new Vector4(
                settings.DopplerStrength,
                settings.RedshiftStrength,
                settings.PhotonRingWidth,
                0.0f));
            material.SetVector(SecondaryImage, new Vector4(
                settings.SecondaryImageHeight,
                settings.SecondaryImageThickness,
                settings.SecondaryImageSpan,
                settings.SecondaryImageIntensity));
            material.SetVector(Environment, new Vector4(settings.StarDensity, settings.Vignette, 0.0f, 0.0f));
        }
    }
}
