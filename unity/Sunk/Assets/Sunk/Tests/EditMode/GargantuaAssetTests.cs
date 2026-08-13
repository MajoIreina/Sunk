using System.Linq;
using NUnit.Framework;
using Sunk.Rendering.Gargantua;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering.Universal;

namespace Sunk.Tests.Rendering.Gargantua
{
    public sealed class GargantuaAssetTests
    {
        private const string PipelinePath = "Assets/Settings/PC_RPAsset.asset";
        private const string RendererPath = "Assets/Sunk/Settings/Sunk_PC_Renderer.asset";
        private const string SettingsPath = "Assets/Sunk/Settings/GargantuaSettings.asset";
        private const string ScenePath = "Assets/Sunk/Scenes/GargantuaPrototype.unity";
        private const string ShaderPath = "Assets/Sunk/Shaders/Gargantua/Gargantua.shader";

        [Test]
        public void ProductAssetsArePresentUnderSunkBoundary()
        {
            Assert.That(AssetDatabase.LoadAssetAtPath<GargantuaSettings>(SettingsPath), Is.Not.Null);
            Assert.That(AssetDatabase.LoadAssetAtPath<SceneAsset>(ScenePath), Is.Not.Null);
            Assert.That(AssetDatabase.LoadAssetAtPath<Shader>(ShaderPath), Is.Not.Null);
        }

        [Test]
        public void ProductRendererContainsOneConfiguredGargantuaFeature()
        {
            GargantuaRendererFeature[] features = AssetDatabase
                .LoadAllAssetsAtPath(RendererPath)
                .OfType<GargantuaRendererFeature>()
                .ToArray();

            Assert.That(features, Has.Length.EqualTo(1));
            Assert.That(features[0].isActive, Is.True);
            Assert.That(features[0].Settings, Is.Not.Null);
        }

        [Test]
        public void PcPipelineUsesProductRendererAsDefault()
        {
            UniversalRenderPipelineAsset pipeline =
                AssetDatabase.LoadAssetAtPath<UniversalRenderPipelineAsset>(PipelinePath);
            Assert.That(pipeline, Is.Not.Null);

            SerializedObject serializedPipeline = new(pipeline);
            SerializedProperty rendererList = serializedPipeline.FindProperty("m_RendererDataList");
            SerializedProperty defaultIndex = serializedPipeline.FindProperty("m_DefaultRendererIndex");
            Assert.That(rendererList, Is.Not.Null);
            Assert.That(rendererList.arraySize, Is.EqualTo(1));
            Assert.That(defaultIndex.intValue, Is.EqualTo(0));
            Assert.That(
                AssetDatabase.GetAssetPath(rendererList.GetArrayElementAtIndex(0).objectReferenceValue),
                Is.EqualTo(RendererPath));
        }
    }
}
