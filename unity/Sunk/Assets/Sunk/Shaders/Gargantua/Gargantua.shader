Shader "Hidden/Sunk/Gargantua"
{
    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" "RenderType" = "Opaque" }
        ZWrite Off
        ZTest Always
        Cull Off

        Pass
        {
            Name "Gargantua"

            HLSLPROGRAM
            #pragma target 3.5
            #pragma vertex Vert
            #pragma fragment Frag
            #pragma multi_compile_instancing

            #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"
            #include "GargantuaCommon.hlsl"

            CBUFFER_START(UnityPerMaterial)
                float _Mass;
                float _Spin;
                float _HorizonRadius;
                float _ApparentShadowRadius;
                float _ScreenShadowRadius;
                float _LensingStrength;
                float4 _DiskRadii;
                float4 _DiskAppearance;
                float4 _Relativity;
                float4 _Integration;
                float4 _DiskGeometry;
                float4 _SecondaryImage;
                float4 _Environment;
            CBUFFER_END

            #include "GargantuaDisk.hlsl"
            #include "GargantuaRayIntegrator.hlsl"

            struct Attributes
            {
                uint vertexID : SV_VertexID;
                UNITY_VERTEX_INPUT_INSTANCE_ID
            };

            struct Varyings
            {
                float4 positionCS : SV_POSITION;
                float2 uv : TEXCOORD0;
                UNITY_VERTEX_OUTPUT_STEREO
            };

            Varyings Vert(Attributes input)
            {
                Varyings output;
                UNITY_SETUP_INSTANCE_ID(input);
                UNITY_INITIALIZE_VERTEX_OUTPUT_STEREO(output);
                output.positionCS = GetFullScreenTriangleVertexPosition(input.vertexID);
                output.uv = GetFullScreenTriangleTexCoord(input.vertexID);
                return output;
            }

            float3 EvaluateStars(float2 sourcePosition)
            {
                float2 starGrid = sourcePosition * 47.0;
                float2 cell = floor(starGrid);
                float2 local = frac(starGrid) - 0.5;
                float seed = SunkHash12(cell);
                float threshold = lerp(0.997, 0.985, _Environment.x);
                float star = step(threshold, seed);
                float size = lerp(0.055, 0.14, SunkHash12(cell + 13.7));
                star *= 1.0 - smoothstep(size * 0.34, size, length(local));
                float brightness = 0.35 + 1.35 * SunkHash12(cell + 4.2);
                float3 tint = lerp(
                    float3(0.58, 0.72, 1.0),
                    float3(1.0, 0.77, 0.52),
                    SunkHash12(cell + 8.9));
                return tint * star * brightness;
            }

            float4 Frag(Varyings input) : SV_Target
            {
                UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX(input);

                float2 position = (input.uv - 0.5) * 2.0;
                position.x *= _ScaledScreenParams.x / max(_ScaledScreenParams.y, 1.0);

                SunkRayTrace trace = SunkTraceKerr(position);
                float shadowRadius = _ScreenShadowRadius;
                float spinOffset = _Spin * shadowRadius * 0.026;
                float2 criticalPosition = position - float2(spinOffset, 0.0);
                criticalPosition.y *= 1.0 + _Spin * 0.016;
                float criticalRadius = length(criticalPosition);
                float2 criticalDirection = criticalPosition / max(criticalRadius, 0.0001);
                float kerrShadowRadius = shadowRadius * (
                    1.0 +
                    0.018 * _Spin * _Spin * (2.0 * criticalDirection.y * criticalDirection.y - 1.0) -
                    0.012 * _Spin * criticalDirection.x);
                float criticalDistance = criticalRadius - kerrShadowRadius;
                float shadowEdge = max(fwidth(criticalDistance), shadowRadius * 0.0035);
                float analyticCapture = 1.0 - smoothstep(-shadowEdge, shadowEdge, criticalDistance);
                float physicalCriticalRadius = 1.5 * max(_HorizonRadius, 0.05) *
                    (1.0 - 0.055 * _Spin * criticalDirection.x);
                float minRadiusDistance = trace.minRadius - physicalCriticalRadius;
                float integratedCapture = 1.0 - smoothstep(
                    -_HorizonRadius * 0.020,
                    _HorizonRadius * 0.055,
                    minRadiusDistance);
                float captureConfidence = max(trace.captured, integratedCapture);
                float backgroundVisibility = 1.0 - max(captureConfidence, analyticCapture * 0.82);
                float resolvedPath = max(trace.escaped, trace.captured);
                float unresolvedVisibility = lerp(0.16, 1.0, resolvedPath);
                backgroundVisibility *= unresolvedVisibility;

                float sourceScale = _ScreenShadowRadius / max(_ApparentShadowRadius, 0.001);
                float2 starCoordinate = trace.sourceCoordinate * sourceScale;
                float3 background = float3(0.0015, 0.0020, 0.0032) + EvaluateStars(starCoordinate);
                float3 color = trace.diskRadiance +
                    background * trace.transmittance * backgroundVisibility;

                // A photon ring is an image of long-lived ray paths, rather than a
                // uniform outline of the analytic shadow. Keep the dominant peak
                // broad enough to resolve at 1.5-3 pixels while the exponentially
                // compressed sub-rings remain only 1-2 pixels wide.
                float pixelWidth = max(fwidth(criticalDistance), shadowRadius * 0.0015);
                float configuredRingWidth = shadowRadius * max(_Relativity.z, 0.004);
                float primaryWidth = max(configuredRingWidth * 0.68, pixelWidth * 1.52);
                float secondaryWidth = max(configuredRingWidth * 0.24, pixelWidth * 0.65);
                float tertiaryWidth = max(configuredRingWidth * 0.12, pixelWidth * 0.47);
                float radialKerrShear = 1.0 + _Spin * criticalDirection.x * 0.08;
                float primaryOffset = max(
                    configuredRingWidth * 1.15,
                    pixelWidth * 1.45) * radialKerrShear;
                float secondaryOffset = max(
                    configuredRingWidth * 0.32,
                    pixelWidth * 0.40) * radialKerrShear;
                float tertiaryOffset = max(
                    configuredRingWidth * 0.15,
                    pixelWidth * 0.22) * radialKerrShear;
                float primaryRing = SunkGaussian(
                    criticalDistance - primaryOffset,
                    primaryWidth);
                float secondaryRing = SunkGaussian(
                    criticalDistance - secondaryOffset,
                    secondaryWidth);
                float tertiaryRing = SunkGaussian(
                    criticalDistance + tertiaryOffset,
                    tertiaryWidth);

                // All three integrator observables must support a ring sample. The
                // progressively stricter gates make higher orders sparse instead of
                // drawing several complete concentric contours.
                float primaryResidency = smoothstep(0.06, 0.52, trace.photonResidency);
                float primaryTurn = smoothstep(0.20, 0.92, trace.totalTurn);
                float primaryWinding = smoothstep(
                    SUNK_PI * 0.06,
                    SUNK_PI * 0.40,
                    trace.orbitalWinding);
                float primaryGate = primaryResidency *
                    lerp(0.22, 1.0, primaryTurn) *
                    lerp(0.18, 1.0, primaryWinding);

                float secondaryGate =
                    smoothstep(0.24, 0.95, trace.photonResidency) *
                    smoothstep(0.68, 1.55, trace.totalTurn) *
                    smoothstep(
                        SUNK_PI * 0.28,
                        SUNK_PI * 0.74,
                        trace.orbitalWinding);
                float tertiaryGate =
                    smoothstep(0.42, 1.20, trace.photonResidency) *
                    smoothstep(0.96, 1.95, trace.totalTurn) *
                    smoothstep(
                        SUNK_PI * 0.44,
                        SUNK_PI * 0.96,
                        trace.orbitalWinding);

                // Sampling noise on the unit critical curve is continuous across
                // the atan2 seam. Each order receives a different source pattern,
                // reproducing partial arcs without painting a screen-space ellipse.
                float coarseArc = SunkNoise(
                    criticalDirection * 3.25 + float2(7.19, -3.41));
                float fineArc = SunkNoise(
                    criticalDirection * 8.60 + float2(-2.73, 11.37));
                float shiftedArc = SunkNoise(
                    criticalDirection.yx * float2(-5.70, 5.70) + float2(4.31, 8.17));
                float primaryArc = lerp(
                    0.14,
                    1.0,
                    smoothstep(0.30, 0.76, coarseArc * 0.72 + fineArc * 0.28));
                float approaching = saturate(0.5 - 0.5 * criticalDirection.x);
                primaryArc = max(primaryArc, smoothstep(0.78, 0.98, approaching) * 0.62);
                float secondaryArc = smoothstep(
                    0.43,
                    0.79,
                    shiftedArc * 0.64 + coarseArc * 0.36);
                float tertiaryArc = smoothstep(
                    0.56,
                    0.86,
                    fineArc * 0.58 + shiftedArc * 0.42);

                float equatorialSource = pow(
                    saturate(1.0 - abs(criticalDirection.y)),
                    0.55);
                float sourceVisibility = lerp(0.07, 1.0, equatorialSource);
                float ring = primaryRing * primaryGate * primaryArc * 0.50 +
                    secondaryRing * secondaryGate * secondaryArc *
                        (_DiskGeometry.z * 1.35) +
                    tertiaryRing * tertiaryGate * tertiaryArc *
                        (_DiskGeometry.z * 0.55);

                // Doppler asymmetry: the approaching limb reaches a cold white HDR
                // peak, while the receding limb falls through amber into dark red.
                float3 ringColor = lerp(
                    float3(0.62, 0.055, 0.012),
                    float3(1.34, 0.46, 0.085),
                    smoothstep(0.05, 0.58, approaching));
                ringColor = lerp(
                    ringColor,
                    float3(2.55, 2.82, 3.16),
                    smoothstep(0.58, 0.97, approaching));
                float dopplerBrightness = lerp(
                    0.10,
                    1.34,
                    pow(approaching, 1.25));
                color += ringColor * ring * sourceVisibility *
                    dopplerBrightness * _DiskAppearance.w;

                float normalizedScreenRadius = length((input.uv - 0.5) * 2.0);
                color *= 1.0 - _Environment.y * smoothstep(0.48, 1.36, normalizedScreenRadius);
                color = max(color, 0.0);
                return float4(color, 1.0);
            }
            ENDHLSL
        }
    }

    Fallback Off
}
