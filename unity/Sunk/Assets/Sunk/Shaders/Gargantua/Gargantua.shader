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

                float pixelWidth = max(fwidth(criticalDistance), shadowRadius * 0.0015);
                float ringWidth = max(shadowRadius * max(_Relativity.z, 0.005), pixelWidth * 1.15);
                float primaryRing = SunkGaussian(criticalDistance - ringWidth * 3.10, ringWidth * 0.92);
                float secondaryRing = SunkGaussian(
                    criticalDistance - ringWidth * 1.32,
                    max(ringWidth * 0.42, pixelWidth * 0.72));
                float tertiaryRing = SunkGaussian(
                    criticalDistance - ringWidth * 0.36,
                    max(ringWidth * 0.20, pixelWidth * 0.54));
                float diskFacing = 0.44 + 0.56 * saturate(
                    0.5 - 0.5 * criticalPosition.x / max(criticalRadius, 0.001));
                float ringAngle = atan2(criticalPosition.y, criticalPosition.x);
                float ringNoise = SunkNoise(float2(ringAngle * 7.2, 2.7));
                float fineStructure = SunkNoise(float2(ringAngle * 18.0 + 4.1, 6.3));
                float ringStructure = 0.34 + 0.48 * ringNoise + 0.18 * fineStructure;
                ringStructure *= 0.88 + 0.12 * sin(ringAngle * 5.0 - _Spin * 1.4);
                float ringEquatorialBias = lerp(
                    0.20,
                    1.0,
                    pow(saturate(1.0 - abs(criticalPosition.y) / max(criticalRadius, 0.001)), 0.46));
                float photonSphereGate = exp2(
                    -minRadiusDistance * minRadiusDistance /
                    max(_HorizonRadius * _HorizonRadius * 0.075, 0.0001) * 1.442695);
                float primaryGate = saturate(
                    photonSphereGate * 0.46 +
                    trace.photonResidency * 0.34 +
                    smoothstep(0.26, 0.82, trace.totalTurn) * 0.28);
                float higherOrderGate = saturate(
                    trace.photonResidency * 0.48 +
                    smoothstep(0.72, 1.85, trace.totalTurn) * 0.42 +
                    smoothstep(SUNK_PI * 0.36, SUNK_PI * 0.92, trace.orbitalWinding) * 0.42);
                float ring = primaryRing * primaryGate +
                    secondaryRing * (_DiskGeometry.z * 1.90) * higherOrderGate +
                    tertiaryRing * (_DiskGeometry.z * 0.82) * higherOrderGate * higherOrderGate;
                float3 ringColor = lerp(
                    float3(1.32, 0.48, 0.10),
                    float3(1.82, 1.46, 1.05),
                    diskFacing);
                color += ringColor * ring * ringStructure *
                    ringEquatorialBias * diskFacing * _DiskAppearance.w;

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
