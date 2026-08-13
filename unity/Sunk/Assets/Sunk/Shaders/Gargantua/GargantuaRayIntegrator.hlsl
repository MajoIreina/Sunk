#ifndef SUNK_GARGANTUA_RAY_INTEGRATOR_INCLUDED
#define SUNK_GARGANTUA_RAY_INTEGRATOR_INCLUDED

static const int SUNK_MAX_RAY_STEPS = 128;

struct SunkRayTrace
{
    float3 position;
    float3 direction;
    float3 diskRadiance;
    float transmittance;
    float minRadius;
    float totalTurn;
    float orbitalWinding;
    float photonResidency;
    float captured;
    float escaped;
    float2 sourceCoordinate;
};

float SunkEvaluateDiskDensity(
    float3 samplePosition,
    float3 diskNormal,
    float innerRadius,
    float outerRadius,
    float halfThickness)
{
    float height = dot(samplePosition, diskNormal);
    float3 radialVector = samplePosition - diskNormal * height;
    float diskRadius = length(radialVector);
    float radial01 = saturate((diskRadius - innerRadius) / max(outerRadius - innerRadius, 0.001));
    float radialFeather = max(halfThickness * 1.35, 0.055);
    float radialMask = smoothstep(innerRadius - radialFeather, innerRadius + radialFeather, diskRadius) *
        (1.0 - smoothstep(outerRadius - radialFeather * 2.0, outerRadius + radialFeather, diskRadius));
    float flaredThickness = halfThickness * lerp(0.82, 1.38, radial01);
    return radialMask * SunkGaussian(height, flaredThickness);
}

float3 SunkDiskNormal()
{
    float inclination = clamp(_DiskGeometry.x, 0.0, SUNK_PI * 0.495);
    return SunkSafeNormalize(float3(0.0, sin(inclination), cos(inclination)));
}

float SunkHigherOrderEnvelope(float2 screenPosition, float projectedMainExtent)
{
    float upperSide = step(0.0, screenPosition.y);
    float heightScale = lerp(0.78, 1.0, upperSide);
    float spanScale = lerp(0.88, 1.0, upperSide);
    float thicknessScale = lerp(0.72, 1.0, upperSide);
    float height = min(_SecondaryImage.x, 1.30) * heightScale;
    float span = min(_SecondaryImage.z, 1.72) * spanScale;
    float halfSpan = max(_ScreenShadowRadius * span, 0.001);
    float normalizedX = abs(screenPosition.x) / halfSpan;
    float endpoint = 1.0 - smoothstep(0.94, 1.01, normalizedX);
    float arc = sqrt(saturate(1.0 - normalizedX * normalizedX));
    float arcHeight = _ScreenShadowRadius *
        (0.10 + (height - 0.10) * arc);
    float halfWidth = max(
        _ScreenShadowRadius * min(_SecondaryImage.y, 0.15) *
            thicknessScale * lerp(0.58, 1.0, saturate(normalizedX)),
        0.0045);
    float signedHeight = lerp(-arcHeight, arcHeight, upperSide);
    float arcBand = SunkGaussian(screenPosition.y - signedHeight, halfWidth);
    float separatedFromMain = smoothstep(
        projectedMainExtent * 0.68,
        projectedMainExtent * 1.22 + halfWidth,
        abs(screenPosition.y));
    return arcBand * endpoint * separatedFromMain * lerp(0.62, 1.0, upperSide);
}

void SunkAccumulateDisk(
    inout SunkRayTrace state,
    float3 previousPosition,
    float3 nextPosition,
    float3 previousDirection,
    float3 nextDirection,
    float stepLength,
    float2 screenPosition,
    float3 diskNormal)
{
    float3 segment = nextPosition - previousPosition;
    float previousHeight = dot(previousPosition, diskNormal);
    float heightDelta = dot(segment, diskNormal);
    float closestT = saturate(
        -previousHeight * heightDelta /
        max(heightDelta * heightDelta, 0.0000001));
    float3 samplePosition = previousPosition + segment * closestT;
    float height = dot(samplePosition, diskNormal);
    float3 radialVector = samplePosition - diskNormal * height;
    float diskRadius = length(radialVector);
    float innerRadius = _DiskRadii.x;
    float outerRadius = _DiskRadii.y;
    float halfThickness = max(_DiskRadii.z, 0.01);
    float radial01 = saturate((diskRadius - innerRadius) / max(outerRadius - innerRadius, 0.001));
    float flaredThickness = halfThickness * lerp(0.82, 1.38, radial01);
    float density = SunkEvaluateDiskDensity(
        samplePosition,
        diskNormal,
        innerRadius,
        outerRadius,
        halfThickness);

    if (density <= 0.0005 || state.transmittance <= 0.002)
    {
        return;
    }

    float previousDensity = SunkEvaluateDiskDensity(
        previousPosition,
        diskNormal,
        innerRadius,
        outerRadius,
        halfThickness);
    float nextDensity = SunkEvaluateDiskDensity(
        nextPosition,
        diskNormal,
        innerRadius,
        outerRadius,
        halfThickness);
    float integratedDensity = (previousDensity + density * 4.0 + nextDensity) / 6.0;
    float opticalDepth = integratedDensity * stepLength *
        max(_DiskGeometry.y, 0.0) / max(flaredThickness * 2.0, 0.025);
    float alpha = saturate(SunkBeerLambert(opticalDepth));
    if (alpha <= 0.0002)
    {
        return;
    }

    float3 xAxis = float3(1.0, 0.0, 0.0);
    float3 zAxis = SunkSafeNormalize(cross(xAxis, diskNormal));
    float3 orbitalDirection = SunkSafeNormalize(cross(radialVector, diskNormal));
    float3 tracedDirection = SunkSafeNormalize(lerp(previousDirection, nextDirection, closestT));
    float3 directionToObserver = -tracedDirection;
    float orbitalToObserver = clamp(dot(orbitalDirection, directionToObserver), -1.0, 1.0);
    float orbitalBeta = clamp(sqrt(0.5 * _HorizonRadius / max(diskRadius, _HorizonRadius * 1.05)), 0.0, 0.68);
    float gamma = rsqrt(max(1.0 - orbitalBeta * orbitalBeta, 0.08));
    float dopplerFactor = rcp(max(gamma * (1.0 - orbitalBeta * orbitalToObserver), 0.25));
    float gravityShift = sqrt(saturate(1.0 - _HorizonRadius / max(diskRadius, _HorizonRadius * 1.01)));
    float azimuth = atan2(dot(radialVector, zAxis), dot(radialVector, xAxis)) / SUNK_PI;
    float2 flowCoordinate = float2(
        log2(max(diskRadius / max(innerRadius, 0.001), 1.0)) * 3.2,
        azimuth * 2.0);

    float3 emission = SunkDiskEmission(
        diskRadius,
        orbitalToObserver,
        dopplerFactor,
        gravityShift,
        flowCoordinate,
        innerRadius,
        outerRadius,
        _DiskRadii.w,
        _DiskAppearance.x,
        _DiskAppearance.y,
        _DiskAppearance.z,
        _Relativity.x,
        _Relativity.y);

    float projectionScale = _ScreenShadowRadius / max(_ApparentShadowRadius, 0.001);
    float projectedMainExtent = (
        outerRadius * abs(cos(_DiskGeometry.x)) +
        halfThickness * 1.8) * projectionScale;
    float separatedImage = smoothstep(
        projectedMainExtent * 0.78,
        projectedMainExtent * 1.38 + 0.004,
        abs(screenPosition.y));
    float windingOrder = state.orbitalWinding / SUNK_PI;
    float lensedOrder = max(smoothstep(0.38, 0.95, state.totalTurn), separatedImage);
    lensedOrder = max(lensedOrder, smoothstep(0.30, 0.72, windingOrder));
    float higherOrder = max(
        smoothstep(1.05, 2.10, state.totalTurn),
        smoothstep(0.78, 1.42, windingOrder));
    float compressedScale = min(_SecondaryImage.w, 0.65) *
        SunkHigherOrderEnvelope(screenPosition, projectedMainExtent);
    compressedScale *= lerp(1.0, min(_DiskGeometry.z, 0.50), higherOrder);

    alpha *= lerp(1.0, compressedScale, lensedOrder);
    state.diskRadiance += state.transmittance * emission * alpha;
    state.transmittance *= 1.0 - alpha;
}

SunkRayTrace SunkTraceKerr(float2 screenPosition)
{
    SunkRayTrace state;
    ZERO_INITIALIZE(SunkRayTrace, state);
    float horizonRadius = max(_HorizonRadius, 0.05);
    float apparentRadius = max(_ApparentShadowRadius, horizonRadius + 0.05);
    float screenRadius = max(_ScreenShadowRadius, 0.01);
    float2 impact = screenPosition * (apparentRadius / screenRadius);
    float farDistance = max(_DiskRadii.y * 1.35, horizonRadius * 14.0);

    state.position = float3(impact, -farDistance);
    state.direction = float3(0.0, 0.0, 1.0);
    state.diskRadiance = 0.0;
    state.transmittance = 1.0;
    state.minRadius = length(state.position);
    state.totalTurn = 0.0;
    state.orbitalWinding = 0.0;
    state.photonResidency = 0.0;
    state.captured = 0.0;
    state.escaped = 0.0;
    state.sourceCoordinate = impact;

    float traceLimit = _DiskRadii.y + horizonRadius * 3.0;
    if (length(impact) > traceLimit)
    {
        state.position = float3(impact, farDistance);
        state.escaped = 1.0;
        return state;
    }

    float3 spinAxis = SunkDiskNormal();
    float lensingScale = max(_LensingStrength, 0.0) / 0.86;
    float massScale = clamp(_Mass / horizonRadius, 0.25, 4.0);
    int requestedSteps = (int)clamp(floor(_Integration.x + 0.5), 24.0, (float)SUNK_MAX_RAY_STEPS);
    float minimumStep = max(_Integration.y, 0.005) * horizonRadius;
    float maximumStep = max(_Integration.z, minimumStep / horizonRadius) * horizonRadius;
    float maximumTurn = clamp(_Integration.w, 0.01, 0.35);

    [loop]
    for (int stepIndex = 0; stepIndex < SUNK_MAX_RAY_STEPS; stepIndex++)
    {
        if (stepIndex >= requestedSteps || state.captured > 0.5 || state.escaped > 0.5)
        {
            break;
        }

        float3 previousPosition = state.position;
        float3 previousDirection = state.direction;
        float radiusSquared = max(dot(previousPosition, previousPosition), 0.000001);
        float radius = sqrt(radiusSquared);
        state.minRadius = min(state.minRadius, radius);
        if (radius <= horizonRadius * 1.025)
        {
            state.captured = 1.0;
            break;
        }

        float inverseRadius = rsqrt(radiusSquared);
        float inverseRadiusCubed = inverseRadius / radiusSquared;
        float inverseRadiusFifth = inverseRadiusCubed / radiusSquared;
        float3 angularMomentum = cross(previousPosition, previousDirection);
        float angularMomentumSquared = dot(angularMomentum, angularMomentum);
        float3 schwarzschildAcceleration =
            -1.5 * horizonRadius * massScale * angularMomentumSquared *
            previousPosition * inverseRadiusFifth * lensingScale;

        float3 radialDirection = previousPosition * inverseRadius;
        float3 gravitomagneticField =
            (3.0 * radialDirection * dot(radialDirection, spinAxis) - spinAxis) *
            inverseRadiusCubed;
        float3 frameDraggingAcceleration =
            0.30 * _Spin * horizonRadius * horizonRadius *
            cross(previousDirection, gravitomagneticField) * lensingScale;
        float3 acceleration = schwarzschildAcceleration + frameDraggingAcceleration;
        acceleration -= previousDirection * dot(acceleration, previousDirection);

        float accelerationMagnitude = length(acceleration);
        float distanceStep = minimumStep + 0.14 * max(radius - horizonRadius, 0.0);
        float turningStep = maximumTurn / max(accelerationMagnitude, 0.0001);
        float stepLength = clamp(min(distanceStep, turningStep), minimumStep, maximumStep);
        float3 turn = acceleration * stepLength;
        turn *= min(1.0, maximumTurn / max(length(turn), 0.00001));

        float3 nextDirection = SunkSafeNormalize(previousDirection + turn);
        float3 nextPosition = previousPosition +
            SunkSafeNormalize(previousDirection + nextDirection) * stepLength;
        state.totalTurn += length(turn);

        float3 previousEquatorial = previousPosition -
            spinAxis * dot(previousPosition, spinAxis);
        float3 nextEquatorial = nextPosition -
            spinAxis * dot(nextPosition, spinAxis);
        float windingDenominator = max(
            length(previousEquatorial) * length(nextEquatorial),
            0.000001);
        float windingSine = dot(
            spinAxis,
            cross(previousEquatorial, nextEquatorial)) / windingDenominator;
        float windingCosine = dot(previousEquatorial, nextEquatorial) / windingDenominator;
        state.orbitalWinding += abs(atan2(windingSine, windingCosine));

        float spinOrientation = clamp(
            dot(cross(previousPosition, previousDirection), spinAxis) /
            max(radius, 0.001),
            -1.0,
            1.0);
        float photonRadius = 1.5 * horizonRadius *
            (1.0 - 0.10 * _Spin * spinOrientation);
        state.photonResidency +=
            SunkGaussian(radius - photonRadius, horizonRadius * 0.18) *
            stepLength / horizonRadius;

        SunkAccumulateDisk(
            state,
            previousPosition,
            nextPosition,
            previousDirection,
            nextDirection,
            stepLength,
            screenPosition,
            spinAxis);

        state.position = nextPosition;
        state.direction = nextDirection;
        float nextRadius = length(nextPosition);
        if (nextRadius <= horizonRadius * 1.025)
        {
            state.captured = 1.0;
        }
        else if (nextRadius >= farDistance && dot(nextPosition, nextDirection) > 0.0)
        {
            state.escaped = 1.0;
        }
    }

    float projectionDistance = 0.0;
    if (state.direction.z > 0.04)
    {
        projectionDistance = max((farDistance - state.position.z) / state.direction.z, 0.0);
    }
    else
    {
        projectionDistance = farDistance;
    }

    projectionDistance = min(projectionDistance, farDistance * 3.0);
    state.sourceCoordinate = state.position.xy + state.direction.xy * projectionDistance;
    return state;
}

#endif
