using NUnit.Framework;
using Sunk.Rendering.Gargantua;
using UnityEngine;

namespace Sunk.Tests.Rendering.Gargantua
{
    public sealed class GargantuaSettingsTests
    {
        private GargantuaSettings settings;

        [SetUp]
        public void SetUp()
        {
            settings = ScriptableObject.CreateInstance<GargantuaSettings>();
        }

        [TearDown]
        public void TearDown()
        {
            Object.DestroyImmediate(settings);
        }

        [Test]
        public void DefaultsKeepPhysicalRadiiInOrder()
        {
            Assert.That(settings.HasValidPhysicalOrdering, Is.True);
            Assert.That(settings.HorizonRadius, Is.LessThan(settings.ApparentShadowRadius));
            Assert.That(settings.ApparentShadowRadius, Is.LessThan(settings.DiskInnerRadius));
            Assert.That(settings.DiskInnerRadius, Is.LessThan(settings.DiskOuterRadius));
        }

        [Test]
        public void DefaultsKeepSecondaryImageCompressed()
        {
            Assert.That(settings.HasCompressedSecondaryImage, Is.True);
            Assert.That(settings.SecondaryImageHeight,
                Is.LessThanOrEqualTo(GargantuaSettings.MaximumSecondaryImageHeight));
            Assert.That(settings.SecondaryImageThickness,
                Is.LessThanOrEqualTo(GargantuaSettings.MaximumSecondaryImageThickness));
            Assert.That(settings.SecondaryImageIntensity,
                Is.LessThanOrEqualTo(GargantuaSettings.MaximumSecondaryImageIntensity));
        }

        [Test]
        public void DefaultsFavorHighSpinGargantuaAppearance()
        {
            Assert.That(settings.Spin, Is.GreaterThanOrEqualTo(0.9f));
            Assert.That(settings.PhotonRingWidth, Is.LessThanOrEqualTo(0.02f));
            Assert.That(settings.DopplerStrength, Is.GreaterThan(0.0f));
            Assert.That(settings.RedshiftStrength, Is.GreaterThan(0.0f));
        }
    }
}
