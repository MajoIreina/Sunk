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
                float4 _SecondaryImage;
                float4 _Environment;
            CBUFFER_END

            #include "GargantuaDisk.hlsl"

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

            float3 EvaluateStars(float2 position, float shadowRadius, float lensingStrength, float spin)
            {
                float radius = max(length(position), shadowRadius * 0.82);
                float2 radial = position / max(radius, 0.001);
                float2 tangent = float2(-radial.y, radial.x);
                float bend = lensingStrength * shadowRadius * shadowRadius / max(radius * radius, 0.04);
                float2 warped = position * (1.0 + bend * 0.58) + tangent * bend * spin * 0.075;

                float2 starGrid = warped * 47.0;
                float2 cell = floor(starGrid);
                float2 local = frac(starGrid) - 0.5;
                float seed = SunkHash12(cell);
                float threshold = lerp(0.997, 0.985, _Environment.x);
                float star = step(threshold, seed);
                float size = lerp(0.055, 0.14, SunkHash12(cell + 13.7));
                star *= 1.0 - smoothstep(size * 0.34, size, length(local));
                float brightness = 0.35 + 1.35 * SunkHash12(cell + 4.2);
                float3 tint = lerp(float3(0.58, 0.72, 1.0), float3(1.0, 0.77, 0.52), SunkHash12(cell + 8.9));
                return tint * star * brightness;
            }

            float EvaluateMainDisk(
                float2 position,
                float shadowRadius,
                float innerRatio,
                float outerRatio,
                out float diskRadius,
                out float2 flowCoordinate)
            {
                float normalizedX = abs(position.x) / max(shadowRadius, 0.001);
                diskRadius = normalizedX * _ApparentShadowRadius;
                float radialMask = smoothstep(innerRatio - 0.06, innerRatio + 0.04, normalizedX) *
                    (1.0 - smoothstep(outerRatio - 0.28, outerRatio, normalizedX));

                float projectedThickness = shadowRadius * _DiskRadii.z;
                float gentleWarp = -shadowRadius * 0.035 *
                    (1.0 - saturate(normalizedX / max(outerRatio, 0.001)));
                float band = SunkGaussian(position.y - gentleWarp, projectedThickness);
                flowCoordinate = float2(normalizedX * 2.6, atan2(position.y - gentleWarp, position.x) / SUNK_PI);
                return radialMask * band;
            }

            float EvaluateLensedImage(
                float2 position,
                float shadowRadius,
                float side,
                out float diskRadius,
                out float2 flowCoordinate)
            {
                float height = min(_SecondaryImage.x, 1.30);
                float thickness = min(_SecondaryImage.y, 0.15);
                float span = min(_SecondaryImage.z, _DiskRadii.y / max(_ApparentShadowRadius, 0.001) - 0.01);
                float sideHeightScale = side > 0.0 ? 1.0 : 0.78;
                float sideSpanScale = side > 0.0 ? 1.0 : 0.88;
                float sideThicknessScale = side > 0.0 ? 1.0 : 0.72;
                float normalizedX = abs(position.x) /
                    max(shadowRadius * span * sideSpanScale, 0.001);
                float inside = 1.0 - step(1.0, normalizedX);
                float arc = sqrt(saturate(1.0 - normalizedX * normalizedX));
                float arcHeight = shadowRadius *
                    (0.10 + (height * sideHeightScale - 0.10) * arc);
                float halfWidth = shadowRadius * thickness * sideThicknessScale *
                    lerp(0.56, 1.0, normalizedX);
                float band = SunkGaussian(position.y - side * arcHeight, halfWidth);

                float physical01 = lerp(0.04, 1.0, pow(saturate(normalizedX), 0.72));
                diskRadius = lerp(_DiskRadii.x, _DiskRadii.y, physical01);
                flowCoordinate = float2(diskRadius * 0.84, side * (0.4 + normalizedX * 1.7));
                return inside * band;
            }

            float4 Frag(Varyings input) : SV_Target
            {
                UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX(input);

                float2 position = (input.uv - 0.5) * 2.0;
                position.x *= _ScaledScreenParams.x / max(_ScaledScreenParams.y, 1.0);

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

                float3 color = float3(0.0015, 0.0020, 0.0032);
                color += EvaluateStars(position, shadowRadius, _LensingStrength, _Spin);

                float innerRatio = _DiskRadii.x / max(_ApparentShadowRadius, 0.001);
                float outerRatio = _DiskRadii.y / max(_ApparentShadowRadius, 0.001);
                float secondarySpan = min(
                    _SecondaryImage.z,
                    _DiskRadii.y / max(_ApparentShadowRadius, 0.001) - 0.01);
                float secondaryDoppler = clamp(
                    position.x / max(shadowRadius * secondarySpan, 0.001),
                    -1.0,
                    1.0);
                float mainDoppler = clamp(
                    position.x / max(shadowRadius * outerRatio, 0.001),
                    -1.0,
                    1.0);
                float diskRadius;
                float2 flowCoordinate;

                float lowerMask = EvaluateLensedImage(position, shadowRadius, -1.0, diskRadius, flowCoordinate);
                float3 lowerEmission = SunkDiskEmission(
                    diskRadius,
                    secondaryDoppler,
                    flowCoordinate,
                    _DiskRadii.x,
                    _DiskRadii.y,
                    _DiskRadii.w,
                    _DiskAppearance.x,
                    _DiskAppearance.y,
                    _DiskAppearance.z,
                    _Relativity.x,
                    _Relativity.y);
                color += lowerEmission * lowerMask * min(_SecondaryImage.w, 0.65) * 0.52;

                float upperMask = EvaluateLensedImage(position, shadowRadius, 1.0, diskRadius, flowCoordinate);
                float3 upperEmission = SunkDiskEmission(
                    diskRadius,
                    secondaryDoppler,
                    flowCoordinate,
                    _DiskRadii.x,
                    _DiskRadii.y,
                    _DiskRadii.w,
                    _DiskAppearance.x,
                    _DiskAppearance.y,
                    _DiskAppearance.z,
                    _Relativity.x,
                    _Relativity.y);
                color += upperEmission * upperMask * min(_SecondaryImage.w, 0.65);

                float mainMask = EvaluateMainDisk(
                    position,
                    shadowRadius,
                    innerRatio,
                    outerRatio,
                    diskRadius,
                    flowCoordinate);
                float3 mainEmission = SunkDiskEmission(
                    diskRadius,
                    mainDoppler,
                    flowCoordinate,
                    _DiskRadii.x,
                    _DiskRadii.y,
                    _DiskRadii.w,
                    _DiskAppearance.x,
                    _DiskAppearance.y,
                    _DiskAppearance.z,
                    _Relativity.x,
                    _Relativity.y);
                color += mainEmission * mainMask;

                float shadowEdge = max(fwidth(criticalDistance), shadowRadius * 0.004);
                float shadow = 1.0 - smoothstep(-shadowEdge, shadowEdge, criticalDistance);
                color *= 1.0 - shadow;

                float ringWidth = shadowRadius * max(_Relativity.z, 0.005);
                float ring = SunkGaussian(criticalDistance - shadowRadius * 0.018, ringWidth);
                float diskFacing = 0.48 + 0.52 * saturate(0.5 - 0.5 * criticalPosition.x / max(criticalRadius, 0.001));
                float ringAngle = atan2(criticalPosition.y, criticalPosition.x);
                float ringStructure = 0.18 + 0.82 * SunkNoise(float2(ringAngle * 5.7, 2.7));
                ringStructure *= 0.82 + 0.18 * sin(ringAngle * 3.0 - _Spin * 1.4);
                float ringEquatorialBias = lerp(
                    0.025,
                    1.0,
                    pow(saturate(1.0 - abs(criticalPosition.y) / max(criticalRadius, 0.001)), 0.42));
                float3 ringColor = lerp(float3(1.45, 0.62, 0.16), float3(1.90, 1.62, 1.20), diskFacing);
                color += ringColor * ring * ringStructure * ringEquatorialBias * diskFacing * _DiskAppearance.w;

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
