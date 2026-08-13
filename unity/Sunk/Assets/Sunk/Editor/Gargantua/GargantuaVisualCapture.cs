using System;
using System.IO;
using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEditor.SceneManagement;
using UnityEngine;

namespace Sunk.Editor.Gargantua
{
    public static class GargantuaVisualCapture
    {
        private const string ScenePath = "Assets/Sunk/Scenes/GargantuaPrototype.unity";
        private const int Width = 1600;
        private const int Height = 900;

        public static void RunBatch()
        {
            EditorSceneManager.OpenScene(ScenePath, OpenSceneMode.Single);
            Camera camera = Camera.main;
            if (camera == null)
            {
                throw new InvalidOperationException("The Gargantua prototype scene has no Main Camera.");
            }

            string repositoryRoot = Path.GetFullPath(
                Path.Combine(Application.dataPath, "..", "..", ".."));
            string outputDirectory = Path.Combine(repositoryRoot, "artifacts");
            string outputPath = Path.Combine(outputDirectory, "gargantua-prototype.png");
            Directory.CreateDirectory(outputDirectory);

            RenderTexture target = new(Width, Height, 24, RenderTextureFormat.ARGB32)
            {
                name = "Sunk Gargantua Visual Validation",
                antiAliasing = 1
            };
            Texture2D capture = new(Width, Height, TextureFormat.RGBA32, false, false);
            RenderTexture previousActive = RenderTexture.active;
            RenderTexture previousTarget = camera.targetTexture;

            try
            {
                target.Create();
                camera.targetTexture = target;
                camera.Render();
                RenderTexture.active = target;
                capture.ReadPixels(new Rect(0, 0, Width, Height), 0, 0, false);
                capture.Apply(false, false);
                File.WriteAllBytes(outputPath, capture.EncodeToPNG());
                Debug.Log($"Sunk Gargantua capture written to {outputPath}.");
            }
            finally
            {
                camera.targetTexture = previousTarget;
                RenderTexture.active = previousActive;
                target.Release();
                UnityEngine.Object.DestroyImmediate(target);
                UnityEngine.Object.DestroyImmediate(capture);
            }
        }

        public static void BuildWindowsBatch()
        {
            string repositoryRoot = Path.GetFullPath(
                Path.Combine(Application.dataPath, "..", "..", ".."));
            string outputDirectory = Path.Combine(repositoryRoot, "artifacts", "windows-player");
            Directory.CreateDirectory(outputDirectory);

            BuildPlayerOptions options = new()
            {
                scenes = new[] { ScenePath },
                locationPathName = Path.Combine(outputDirectory, "sunk.exe"),
                target = BuildTarget.StandaloneWindows64,
                options = BuildOptions.StrictMode
            };

            BuildReport report = BuildPipeline.BuildPlayer(options);
            if (report.summary.result != BuildResult.Succeeded)
            {
                throw new InvalidOperationException(
                    $"Sunk Windows player build failed: {report.summary.result}.");
            }

            Debug.Log(
                $"Sunk Windows player written to {options.locationPathName} " +
                $"({report.summary.totalSize} bytes).");
        }
    }
}
