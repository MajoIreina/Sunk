using System;
using System.Collections;
using System.IO;
using UnityEngine;

namespace Sunk.Demo
{
    public sealed class SunkDemoControl : MonoBehaviour
    {
        private const string CaptureArgument = "-sunkCapture";
        private const string QuitAfterCaptureArgument = "-sunkQuitAfterCapture";
        private const int CaptureWidth = 1600;
        private const int CaptureHeight = 900;
        private const int ResolutionWaitFrames = 120;

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.BeforeSceneLoad)]
        private static void Install()
        {
            GameObject host = new("Sunk Demo Control");
            DontDestroyOnLoad(host);
            host.AddComponent<SunkDemoControl>();
        }

        private void Start()
        {
            string[] arguments = Environment.GetCommandLineArgs();
            bool captureRequested = HasArgument(arguments, CaptureArgument);
            string capturePath = ReadArgumentValue(arguments, CaptureArgument);
            bool quitAfterCapture = HasArgument(arguments, QuitAfterCaptureArgument);
            if (!captureRequested)
            {
                return;
            }

            if (capturePath == null)
            {
                if (quitAfterCapture)
                {
                    Application.Quit(1);
                }

                return;
            }

            StartCoroutine(CapturePlayerFrame(capturePath, quitAfterCapture));
        }

        private void Update()
        {
            if (Input.GetKeyDown(KeyCode.Escape))
            {
                Application.Quit();
            }
        }

        private static IEnumerator CapturePlayerFrame(string requestedPath, bool quitAfterCapture)
        {
            string outputPath;
            try
            {
                outputPath = Path.GetFullPath(requestedPath);
                string outputDirectory = Path.GetDirectoryName(outputPath);
                if (!string.IsNullOrEmpty(outputDirectory))
                {
                    Directory.CreateDirectory(outputDirectory);
                }
            }
            catch (Exception exception)
            {
                FinishCaptureWithError(
                    $"Sunk could not prepare the requested capture path '{requestedPath}'.",
                    exception,
                    quitAfterCapture);
                yield break;
            }

            Screen.SetResolution(CaptureWidth, CaptureHeight, FullScreenMode.Windowed);

            int waitFrames = 0;
            while ((Screen.width != CaptureWidth || Screen.height != CaptureHeight) &&
                   waitFrames < ResolutionWaitFrames)
            {
                waitFrames++;
                yield return null;
            }

            if (Screen.width != CaptureWidth || Screen.height != CaptureHeight)
            {
                FinishCaptureWithError(
                    $"Sunk could not establish the {CaptureWidth}x{CaptureHeight} capture surface; " +
                    $"the player reported {Screen.width}x{Screen.height}.",
                    null,
                    quitAfterCapture);
                yield break;
            }

            // Let the scene and render pipeline settle before reading the completed back buffer.
            yield return null;
            yield return new WaitForEndOfFrame();

            Texture2D capture = null;
            try
            {
                capture = new Texture2D(CaptureWidth, CaptureHeight, TextureFormat.RGB24, false, false);
                capture.ReadPixels(new Rect(0, 0, CaptureWidth, CaptureHeight), 0, 0, false);
                capture.Apply(false, false);
                byte[] pngData = ImageConversion.EncodeToPNG(capture);
                File.WriteAllBytes(outputPath, pngData);
                Debug.Log(
                    $"Sunk player capture written to {outputPath} " +
                    $"({CaptureWidth}x{CaptureHeight}).");
            }
            catch (Exception exception)
            {
                FinishCaptureWithError(
                    $"Sunk could not write the player capture to '{outputPath}'.",
                    exception,
                    quitAfterCapture);
                yield break;
            }
            finally
            {
                if (capture != null)
                {
                    Destroy(capture);
                }
            }

            if (quitAfterCapture)
            {
                Application.Quit(0);
            }
        }

        private static string ReadArgumentValue(string[] arguments, string argumentName)
        {
            for (int index = 0; index < arguments.Length; index++)
            {
                if (!string.Equals(arguments[index], argumentName, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                if (index + 1 >= arguments.Length ||
                    string.IsNullOrWhiteSpace(arguments[index + 1]) ||
                    arguments[index + 1].StartsWith("-", StringComparison.Ordinal))
                {
                    Debug.LogError($"Sunk requires a file path after {argumentName}.");
                    return null;
                }

                return arguments[index + 1];
            }

            return null;
        }

        private static bool HasArgument(string[] arguments, string argumentName)
        {
            foreach (string argument in arguments)
            {
                if (string.Equals(argument, argumentName, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }

            return false;
        }

        private static void FinishCaptureWithError(
            string message,
            Exception exception,
            bool quitAfterCapture)
        {
            if (exception == null)
            {
                Debug.LogError(message);
            }
            else
            {
                Debug.LogError($"{message}\n{exception}");
            }

            if (quitAfterCapture)
            {
                Application.Quit(1);
            }
        }
    }
}
