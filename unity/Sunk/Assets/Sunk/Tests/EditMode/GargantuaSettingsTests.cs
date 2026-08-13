using System.Reflection;
using NUnit.Framework;
using Sunk.Rendering.Gargantua;
using UnityEditor;
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
            Assert.That(settings.HorizonRadius, Is.LessThan(settings.DiskInnerRadius));
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
            Assert.That(settings.DiskInclination, Is.GreaterThanOrEqualTo(80.0f));
            Assert.That(settings.DiskInnerRadius, Is.LessThan(settings.ApparentShadowRadius));
            Assert.That(settings.DiskOuterRadius, Is.LessThanOrEqualTo(9.5f));
            Assert.That(settings.DiskHalfThickness, Is.LessThanOrEqualTo(0.05f));
            Assert.That(settings.Turbulence, Is.GreaterThanOrEqualTo(0.9f));
            Assert.That(settings.HigherOrderIntensity, Is.LessThan(settings.SecondaryImageIntensity));
        }

        [Test]
        public void DefaultsUseStableRayIntegrationBounds()
        {
            Assert.That(settings.HasValidRayIntegration, Is.True);
            Assert.That(settings.RaySteps,
                Is.InRange(GargantuaSettings.MinimumRaySteps, GargantuaSettings.MaximumRaySteps));
            Assert.That(settings.RaySteps, Is.LessThanOrEqualTo(96));
            Assert.That(settings.MinStep, Is.GreaterThan(0.0f));
            Assert.That(settings.MaxStep, Is.GreaterThan(settings.MinStep));
            Assert.That(settings.MaxTurn,
                Is.InRange(GargantuaSettings.MinimumRayTurn, GargantuaSettings.MaximumRayTurn));
        }

        [Test]
        public void DefaultsKeepDiskGeometryPhysicalAndCompact()
        {
            Assert.That(settings.HasValidDiskGeometry, Is.True);
            Assert.That(settings.DiskInclination,
                Is.InRange(0.0f, GargantuaSettings.MaximumDiskInclination));
            Assert.That(settings.DiskOpacity, Is.InRange(0.0f, 1.0f));
            Assert.That(settings.HigherOrderIntensity,
                Is.InRange(0.0f, GargantuaSettings.MaximumHigherOrderIntensity));
        }

        [Test]
        public void OnValidateClampsIntegrationAndDiskGeometry()
        {
            SerializedObject serializedSettings = new(settings);
            serializedSettings.FindProperty("raySteps").intValue = 512;
            serializedSettings.FindProperty("minStep").floatValue = 0.10f;
            serializedSettings.FindProperty("maxStep").floatValue = 0.01f;
            serializedSettings.FindProperty("maxTurn").floatValue = 10.0f;
            serializedSettings.FindProperty("diskInclination").floatValue = 180.0f;
            serializedSettings.FindProperty("diskOpacity").floatValue = -1.0f;
            serializedSettings.FindProperty("higherOrderIntensity").floatValue = 4.0f;
            serializedSettings.ApplyModifiedPropertiesWithoutUndo();

            MethodInfo onValidate = typeof(GargantuaSettings).GetMethod(
                "OnValidate",
                BindingFlags.Instance | BindingFlags.NonPublic);
            Assert.That(onValidate, Is.Not.Null);
            onValidate.Invoke(settings, null);

            Assert.That(settings.RaySteps, Is.EqualTo(GargantuaSettings.MaximumRaySteps));
            Assert.That(settings.MaxStep, Is.EqualTo(settings.MinStep));
            Assert.That(settings.MaxTurn, Is.EqualTo(GargantuaSettings.MaximumRayTurn));
            Assert.That(settings.DiskInclination,
                Is.EqualTo(GargantuaSettings.MaximumDiskInclination));
            Assert.That(settings.DiskOpacity, Is.Zero);
            Assert.That(settings.HigherOrderIntensity,
                Is.EqualTo(GargantuaSettings.MaximumHigherOrderIntensity));
            Assert.That(settings.HasValidRayIntegration, Is.True);
            Assert.That(settings.HasValidDiskGeometry, Is.True);
        }
    }
}
