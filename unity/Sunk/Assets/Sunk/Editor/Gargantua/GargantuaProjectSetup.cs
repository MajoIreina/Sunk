using System;
using System.Linq;
using Sunk.Rendering.Gargantua;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering.Universal;

namespace Sunk.Editor.Gargantua
{
    public static class GargantuaProjectSetup
    {
        private const string PipelineAssetPath = "Assets/Settings/PC_RPAsset.asset";
        private const string TemplateRendererPath = "Assets/Settings/PC_Renderer.asset";
        private const string ProductRendererPath = "Assets/Sunk/Settings/Sunk_PC_Renderer.asset";
        private const string ShaderPath = "Assets/Sunk/Shaders/Gargantua/Gargantua.shader";
        private const string SettingsPath = "Assets/Sunk/Settings/GargantuaSettings.asset";
        private const string TemplateScenePath = "Assets/Scenes/SampleScene.unity";
        private const string PrototypeScenePath = "Assets/Sunk/Scenes/GargantuaPrototype.unity";

        [MenuItem("Sunk/Setup Gargantua Prototype")]
        public static void RunFromMenu()
        {
            if (!EditorSceneManagerBridge.SaveCurrentModifiedScenesIfUserWantsTo())
            {
                return;
            }

            Run();
        }

        public static void RunBatch()
        {
            Run();
        }

        private static void Run()
        {
            EnsureFolder("Assets/Sunk/Settings");
            EnsureFolder("Assets/Sunk/Scenes");

            GargantuaSettings settings = LoadOrCreateSettings();
            Shader shader = AssetDatabase.LoadAssetAtPath<Shader>(ShaderPath);
            if (shader == null)
            {
                throw new InvalidOperationException($"Gargantua shader is missing at {ShaderPath}.");
            }

            ScriptableRendererData rendererData = LoadOrCreateProductRenderer();
            ConnectPipelineToProductRenderer(rendererData);

            GargantuaRendererFeature feature = LoadOrCreateRendererFeature(rendererData);
            bool featureChanged = false;
            if (feature.name != "Sunk Gargantua")
            {
                feature.name = "Sunk Gargantua";
                featureChanged = true;
            }

            featureChanged |= feature.Configure(settings, shader);
            if (!feature.isActive)
            {
                feature.SetActive(true);
                featureChanged = true;
            }

            bool rendererChanged = SynchronizeRendererFeatureMap(rendererData, feature);

            EnsurePrototypeScene();
            EditorBuildSettingsScene[] buildScenes = EditorBuildSettings.scenes;
            if (buildScenes.Length != 1 ||
                !buildScenes[0].enabled ||
                buildScenes[0].path != PrototypeScenePath)
            {
                EditorBuildSettings.scenes = new[]
                {
                    new EditorBuildSettingsScene(PrototypeScenePath, true)
                };
            }

            if (rendererChanged)
            {
                EditorUtility.SetDirty(rendererData);
                rendererData.SetDirty();
            }

            if (featureChanged)
            {
                EditorUtility.SetDirty(feature);
            }

            AssetDatabase.SaveAssets();
            AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);

            Debug.Log("Sunk Gargantua prototype setup completed.");
        }

        private static GargantuaSettings LoadOrCreateSettings()
        {
            GargantuaSettings settings = AssetDatabase.LoadAssetAtPath<GargantuaSettings>(SettingsPath);
            if (settings != null)
            {
                return settings;
            }

            settings = ScriptableObject.CreateInstance<GargantuaSettings>();
            settings.name = "GargantuaSettings";
            AssetDatabase.CreateAsset(settings, SettingsPath);
            return settings;
        }

        private static GargantuaRendererFeature LoadOrCreateRendererFeature(ScriptableRendererData rendererData)
        {
            GargantuaRendererFeature[] subAssets = AssetDatabase
                .LoadAllAssetsAtPath(ProductRendererPath)
                .OfType<GargantuaRendererFeature>()
                .ToArray();

            GargantuaRendererFeature[] listedFeatures = rendererData.rendererFeatures
                .OfType<GargantuaRendererFeature>()
                .ToArray();

            GargantuaRendererFeature[] candidates = subAssets
                .Concat(listedFeatures)
                .Distinct()
                .ToArray();

            if (candidates.Length > 1)
            {
                throw new InvalidOperationException("PC_Renderer contains multiple Gargantua renderer features.");
            }

            if (rendererData.rendererFeatures.Any(item => item == null))
            {
                throw new InvalidOperationException("PC_Renderer contains a missing renderer feature reference.");
            }

            if (candidates.Length == 1)
            {
                return candidates[0];
            }

            GargantuaRendererFeature feature = ScriptableObject.CreateInstance<GargantuaRendererFeature>();
            feature.name = "Sunk Gargantua";
            AssetDatabase.AddObjectToAsset(feature, rendererData);
            return feature;
        }

        private static ScriptableRendererData LoadOrCreateProductRenderer()
        {
            ScriptableRendererData rendererData =
                AssetDatabase.LoadAssetAtPath<ScriptableRendererData>(ProductRendererPath);
            if (rendererData != null)
            {
                return rendererData;
            }

            if (AssetDatabase.LoadAssetAtPath<ScriptableRendererData>(TemplateRendererPath) == null)
            {
                throw new InvalidOperationException(
                    $"PC renderer template is missing at {TemplateRendererPath}.");
            }

            if (!AssetDatabase.CopyAsset(TemplateRendererPath, ProductRendererPath))
            {
                throw new InvalidOperationException("Unity could not create the Sunk PC renderer asset.");
            }

            AssetDatabase.ImportAsset(ProductRendererPath, ImportAssetOptions.ForceSynchronousImport);
            rendererData = AssetDatabase.LoadAssetAtPath<ScriptableRendererData>(ProductRendererPath);
            if (rendererData == null)
            {
                throw new InvalidOperationException("The Sunk PC renderer asset could not be verified.");
            }

            rendererData.name = "Sunk_PC_Renderer";
            EditorUtility.SetDirty(rendererData);
            return rendererData;
        }

        private static void ConnectPipelineToProductRenderer(ScriptableRendererData rendererData)
        {
            UniversalRenderPipelineAsset pipelineAsset =
                AssetDatabase.LoadAssetAtPath<UniversalRenderPipelineAsset>(PipelineAssetPath);
            if (pipelineAsset == null)
            {
                throw new InvalidOperationException($"PC pipeline asset is missing at {PipelineAssetPath}.");
            }

            SerializedObject serializedPipeline = new(pipelineAsset);
            SerializedProperty rendererList = serializedPipeline.FindProperty("m_RendererDataList");
            SerializedProperty defaultRenderer = serializedPipeline.FindProperty("m_DefaultRendererIndex");
            if (rendererList == null || !rendererList.isArray || defaultRenderer == null)
            {
                throw new InvalidOperationException("URP pipeline renderer serialization layout is unsupported.");
            }

            serializedPipeline.Update();
            if (rendererList.arraySize != 1 || defaultRenderer.intValue != 0)
            {
                throw new InvalidOperationException(
                    "PC_RPAsset no longer has the expected single default renderer configuration.");
            }

            SerializedProperty defaultRendererReference = rendererList.GetArrayElementAtIndex(0);
            if (defaultRendererReference.objectReferenceValue != rendererData)
            {
                defaultRendererReference.objectReferenceValue = rendererData;
                serializedPipeline.ApplyModifiedPropertiesWithoutUndo();
                EditorUtility.SetDirty(pipelineAsset);
            }
        }

        private static bool SynchronizeRendererFeatureMap(
            ScriptableRendererData rendererData,
            GargantuaRendererFeature gargantuaFeature)
        {
            SerializedObject serializedRenderer = new(rendererData);
            SerializedProperty features = serializedRenderer.FindProperty("m_RendererFeatures");
            SerializedProperty featureMap = serializedRenderer.FindProperty("m_RendererFeatureMap");
            if (features == null || featureMap == null || !features.isArray || !featureMap.isArray)
            {
                throw new InvalidOperationException("URP renderer feature serialization layout is unsupported.");
            }

            serializedRenderer.Update();

            bool containsFeature = false;
            bool changed = false;
            for (int index = 0; index < features.arraySize; index++)
            {
                UnityEngine.Object item = features.GetArrayElementAtIndex(index).objectReferenceValue;
                if (item == null)
                {
                    throw new InvalidOperationException("PC_Renderer contains a null renderer feature entry.");
                }

                containsFeature |= item == gargantuaFeature;
            }

            if (!containsFeature)
            {
                int newIndex = features.arraySize;
                features.arraySize++;
                features.GetArrayElementAtIndex(newIndex).objectReferenceValue = gargantuaFeature;
                changed = true;
            }

            if (featureMap.arraySize != features.arraySize)
            {
                featureMap.arraySize = features.arraySize;
                changed = true;
            }

            for (int index = 0; index < features.arraySize; index++)
            {
                UnityEngine.Object feature = features.GetArrayElementAtIndex(index).objectReferenceValue;
                if (!AssetDatabase.TryGetGUIDAndLocalFileIdentifier(feature, out _, out long localId))
                {
                    throw new InvalidOperationException($"Unable to resolve renderer feature at index {index}.");
                }

                SerializedProperty mapEntry = featureMap.GetArrayElementAtIndex(index);
                if (mapEntry.longValue != localId)
                {
                    mapEntry.longValue = localId;
                    changed = true;
                }
            }

            if (changed)
            {
                serializedRenderer.ApplyModifiedPropertiesWithoutUndo();
            }

            return changed;
        }

        private static void EnsurePrototypeScene()
        {
            if (AssetDatabase.LoadAssetAtPath<SceneAsset>(PrototypeScenePath) != null)
            {
                return;
            }

            if (AssetDatabase.LoadAssetAtPath<SceneAsset>(TemplateScenePath) == null)
            {
                throw new InvalidOperationException($"Template scene is missing at {TemplateScenePath}.");
            }

            if (!AssetDatabase.CopyAsset(TemplateScenePath, PrototypeScenePath))
            {
                throw new InvalidOperationException("Unity could not create the Gargantua prototype scene.");
            }

            AssetDatabase.ImportAsset(PrototypeScenePath, ImportAssetOptions.ForceSynchronousImport);
            if (AssetDatabase.LoadAssetAtPath<SceneAsset>(PrototypeScenePath) == null ||
                string.IsNullOrEmpty(AssetDatabase.AssetPathToGUID(PrototypeScenePath)))
            {
                throw new InvalidOperationException("Gargantua prototype scene could not be verified.");
            }
        }

        private static void EnsureFolder(string path)
        {
            if (AssetDatabase.IsValidFolder(path))
            {
                return;
            }

            string[] segments = path.Split('/');
            string current = segments[0];
            for (int index = 1; index < segments.Length; index++)
            {
                string next = $"{current}/{segments[index]}";
                if (!AssetDatabase.IsValidFolder(next))
                {
                    AssetDatabase.CreateFolder(current, segments[index]);
                }

                current = next;
            }
        }

        private static class EditorSceneManagerBridge
        {
            public static bool SaveCurrentModifiedScenesIfUserWantsTo()
            {
                return UnityEditor.SceneManagement.EditorSceneManager
                    .SaveCurrentModifiedScenesIfUserWantsTo();
            }
        }
    }
}
