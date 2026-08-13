using UnityEngine;

namespace Sunk.Rendering.Gargantua
{
    [CreateAssetMenu(fileName = "GargantuaSettings", menuName = "Sunk/Rendering/Gargantua Settings")]
    public sealed class GargantuaSettings : ScriptableObject
    {
        public const float MaximumSecondaryImageHeight = 1.30f;
        public const float MaximumSecondaryImageThickness = 0.15f;
        public const float MaximumSecondaryImageIntensity = 0.65f;

        [Header("Black Hole")]
        [SerializeField, Min(0.1f)] private float mass = 1.0f;
        [SerializeField, Range(0.0f, 0.998f)] private float spin = 0.92f;
        [SerializeField, Min(0.1f)] private float horizonRadius = 1.0f;
        [SerializeField, Min(0.1f)] private float apparentShadowRadius = 2.60f;
        [SerializeField, Range(0.1f, 0.8f)] private float screenShadowRadius = 0.37f;
        [SerializeField, Range(0.0f, 2.0f)] private float lensingStrength = 0.86f;

        [Header("Accretion Disk")]
        [SerializeField, Min(0.1f)] private float diskInnerRadius = 3.20f;
        [SerializeField, Min(0.2f)] private float diskOuterRadius = 10.40f;
        [SerializeField, Range(0.01f, 0.25f)] private float diskHalfThickness = 0.08f;
        [SerializeField, Range(1000.0f, 20000.0f)] private float diskTemperature = 9200.0f;
        [SerializeField, Range(0.0f, 3.0f)] private float diskEmission = 1.15f;
        [SerializeField, Range(0.0f, 2.0f)] private float turbulence = 0.55f;
        [SerializeField, Range(0.0f, 2.0f)] private float rotationSpeed = 0.08f;

        [Header("Relativistic Appearance")]
        [SerializeField, Range(0.0f, 2.0f)] private float dopplerStrength = 0.82f;
        [SerializeField, Range(0.0f, 1.0f)] private float redshiftStrength = 0.72f;
        [SerializeField, Range(0.0f, 3.0f)] private float photonRingIntensity = 0.46f;
        [SerializeField, Range(0.005f, 0.08f)] private float photonRingWidth = 0.009f;

        [Header("Compressed Lensed Image")]
        [Tooltip("Highest point of the upper image, measured in apparent shadow radii.")]
        [SerializeField, Range(1.05f, MaximumSecondaryImageHeight)]
        private float secondaryImageHeight = 1.08f;

        [Tooltip("Half thickness of the lensed image, measured in apparent shadow radii.")]
        [SerializeField, Range(0.04f, MaximumSecondaryImageThickness)]
        private float secondaryImageThickness = 0.045f;

        [Tooltip("Horizontal half-span of the lensed image, measured in apparent shadow radii.")]
        [SerializeField, Range(1.5f, 3.2f)] private float secondaryImageSpan = 1.72f;
        [SerializeField, Range(0.0f, MaximumSecondaryImageIntensity)]
        private float secondaryImageIntensity = 0.28f;

        [Header("Environment")]
        [SerializeField, Range(0.0f, 1.0f)] private float starDensity = 0.28f;
        [SerializeField, Range(0.0f, 1.0f)] private float vignette = 0.32f;

        public float Mass => mass;
        public float Spin => spin;
        public float HorizonRadius => horizonRadius;
        public float ApparentShadowRadius => apparentShadowRadius;
        public float ScreenShadowRadius => screenShadowRadius;
        public float LensingStrength => lensingStrength;
        public float DiskInnerRadius => diskInnerRadius;
        public float DiskOuterRadius => diskOuterRadius;
        public float DiskHalfThickness => diskHalfThickness;
        public float DiskTemperature => diskTemperature;
        public float DiskEmission => diskEmission;
        public float Turbulence => turbulence;
        public float RotationSpeed => rotationSpeed;
        public float DopplerStrength => dopplerStrength;
        public float RedshiftStrength => redshiftStrength;
        public float PhotonRingIntensity => photonRingIntensity;
        public float PhotonRingWidth => photonRingWidth;
        public float SecondaryImageHeight => secondaryImageHeight;
        public float SecondaryImageThickness => secondaryImageThickness;
        public float SecondaryImageSpan => secondaryImageSpan;
        public float SecondaryImageIntensity => secondaryImageIntensity;
        public float StarDensity => starDensity;
        public float Vignette => vignette;

        public bool HasValidPhysicalOrdering =>
            mass > 0.0f &&
            horizonRadius > 0.0f &&
            apparentShadowRadius > horizonRadius &&
            diskInnerRadius > apparentShadowRadius &&
            diskOuterRadius > diskInnerRadius;

        public bool HasCompressedSecondaryImage =>
            secondaryImageHeight <= MaximumSecondaryImageHeight &&
            secondaryImageThickness <= MaximumSecondaryImageThickness &&
            secondaryImageIntensity <= MaximumSecondaryImageIntensity &&
            secondaryImageSpan * apparentShadowRadius < diskOuterRadius;

        private void OnValidate()
        {
            mass = Mathf.Max(0.1f, mass);
            horizonRadius = Mathf.Max(0.1f, horizonRadius);
            apparentShadowRadius = Mathf.Max(horizonRadius + 0.1f, apparentShadowRadius);
            diskInnerRadius = Mathf.Max(apparentShadowRadius + 0.1f, diskInnerRadius);
            diskOuterRadius = Mathf.Max(diskInnerRadius + 0.1f, diskOuterRadius);
            secondaryImageHeight = Mathf.Min(secondaryImageHeight, MaximumSecondaryImageHeight);
            secondaryImageThickness = Mathf.Min(secondaryImageThickness, MaximumSecondaryImageThickness);
            secondaryImageIntensity = Mathf.Min(secondaryImageIntensity, MaximumSecondaryImageIntensity);
            secondaryImageSpan = Mathf.Min(secondaryImageSpan, diskOuterRadius / apparentShadowRadius - 0.01f);
        }
    }
}
