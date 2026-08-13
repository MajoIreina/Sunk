#ifndef SUNK_GARGANTUA_RAY_INTEGRATOR_INCLUDED
#define SUNK_GARGANTUA_RAY_INTEGRATOR_INCLUDED

static const int SUNK_MAX_RAY_STEPS = 192;

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
    float diskPlaneCrossings;
    float diskSide;
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

float3 SunkRayAcceleration(
    float3 position,
    float3 direction,
    float3 spinAxis,
    float horizonRadius,
    float massScale,
    float lensingScale)
{
    float radiusSquared = max(dot(position, position), 0.000001);
    float radius = sqrt(radiusSquared);
    float inverseRadius = rsqrt(radiusSquared);
    float inverseRadiusCubed = inverseRadius / radiusSquared;
    float inverseRadiusFifth = inverseRadiusCubed / radiusSquared;
    float3 angularMomentum = cross(position, direction);
    float angularMomentumSquared = dot(angularMomentum, angularMomentum);
    float3 schwarzschildAcceleration =
        -1.5 * horizonRadius * massScale * angularMomentumSquared *
        position * inverseRadiusFifth * lensingScale;

    float3 radialDirection = position * inverseRadius;
    float3 gravitomagneticField =
        (3.0 * radialDirection * dot(radialDirection, spinAxis) - spinAxis) *
        inverseRadiusCubed;
    float3 frameDraggingAcceleration =
        0.30 * _Spin * horizonRadius * horizonRadius *
        cross(direction, gravitomagneticField) * lensingScale;
    float3 acceleration = schwarzschildAcceleration + frameDraggingAcceleration;
    return acceleration - direction * dot(acceleration, direction);
}

float SunkDiskOrderAttenuation(
    SunkRayTrace state,
    float diskPassage,
    float diskRadius,
    float crossingOrientation)
{
    // Plane passages form the discrete transfer-function order. Bending, azimuthal
    // winding and photon-orbit dwell only provide continuous confidence for a highly
    // lensed first passage; none of these quantities creates a screen-space image.
    float discreteOrder = max(floor(diskPassage + 0.01) - 1.0, 0.0);
    float windingOrder = state.orbitalWinding / SUNK_PI;
    float secondaryBend = smoothstep(0.24, 0.62, state.totalTurn);
    float secondaryOrbit = max(
        smoothstep(0.16, 0.52, windingOrder),
        smoothstep(0.08, 0.48, state.photonResidency));
    float trajectorySecondary = max(
        secondaryBend,
        secondaryOrbit * smoothstep(0.14, 0.40, state.totalTurn));
    float secondaryOrder = max(step(0.5, discreteOrder), trajectorySecondary);

    float higherBend = smoothstep(0.78, 1.72, state.totalTurn);
    float higherOrbit = max(
        smoothstep(0.76, 1.46, windingOrder),
        smoothstep(0.68, 1.55, state.photonResidency));
    float trajectoryHigher = higherBend * lerp(0.42, 1.0, higherOrbit);
    float higherOrder = max(step(1.5, discreteOrder), trajectoryHigher);

    // Outer disk radii subtend the overly tall, broad transfer-function images.
    // Their contribution falls naturally with emission radius, while the inner
    // disk remains continuous all the way across the critical region.
    float innerRadius = max(_DiskRadii.x, 0.001);
    float radialRatio = saturate(innerRadius / max(diskRadius, innerRadius));
    float secondaryRadialTransfer = pow(radialRatio, 2.35);
    float higherRadialTransfer = pow(radialRatio, 3.00);

    // Each additional plane passage and accumulated orbit loses flux
    // exponentially. The first secondary passage is preserved; later orders and
    // long-lived photon-orbit paths converge rapidly toward the critical curve.
    float orderDecay = exp2(-0.72 * max(discreteOrder - 1.0, 0.0));
    float windingDecay = exp2(-0.48 * max(windingOrder - 0.24, 0.0));
    float pathDecay = orderDecay * windingDecay;

    // Rays are traced away from the observer. The sign of their disk-normal motion
    // identifies which optically thick disk face is seen. Returning passages are
    // slightly more self-obscured, producing physical upper/lower asymmetry without
    // referring to screen coordinates.
    float positiveFaceToObserver = step(crossingOrientation, 0.0);
    float faceVisibility = lerp(0.42, 1.0, positiveFaceToObserver);
    float returningPassage = step(1.5, diskPassage);
    float sequenceVisibility = lerp(
        1.0,
        lerp(0.64, 1.0, positiveFaceToObserver),
        returningPassage);
    float orientedTransfer = faceVisibility * sequenceVisibility;

    float secondaryScale = min(_SecondaryImage.w, 0.65);
    float higherScale = min(_DiskGeometry.z, 0.50);
    float secondaryTransfer = secondaryScale * secondaryRadialTransfer *
        pathDecay * orientedTransfer;
    float higherTransfer = higherScale * higherRadialTransfer *
        pathDecay * orientedTransfer;
    return lerp(1.0, secondaryTransfer, saturate(secondaryOrder)) *
        lerp(1.0, higherTransfer, saturate(higherOrder));
}

void SunkAccumulateDisk(
    inout SunkRayTrace state,
    float3 previousPosition,
    float3 nextPosition,
    float3 previousDirection,
    float3 nextDirection,
    float stepLength,
    float3 diskNormal)
{
    float3 segment = nextPosition - previousPosition;
    float previousHeight = dot(previousPosition, diskNormal);
    float heightDelta = dot(segment, diskNormal);
    float nextHeight = previousHeight + heightDelta;
    float closestT = abs(heightDelta) > 0.00001
        ? saturate(-previousHeight / heightDelta)
        : 0.5;
    float3 closestPosition = previousPosition + segment * closestT;
    float closestHeight = dot(closestPosition, diskNormal);
    float3 closestRadial = closestPosition - diskNormal * closestHeight;
    float closestRadius = length(closestRadial);
    float innerRadius = _DiskRadii.x;
    float outerRadius = _DiskRadii.y;
    float halfThickness = max(_DiskRadii.z, 0.01);
    float radial01 = saturate((closestRadius - innerRadius) / max(outerRadius - innerRadius, 0.001));
    float flaredThickness = halfThickness * lerp(0.82, 1.38, radial01);

    // Count actual equatorial-plane passages once, independently of the number of
    // volume samples taken through the disk thickness. Crossings through the central
    // cavity still advance the transfer-function order, provided they occur inside
    // the disk's outer extent.
    float previousSide = previousHeight >= 0.0 ? 1.0 : -1.0;
    if (abs(state.diskSide) < 0.5)
    {
        state.diskSide = previousSide;
    }

    float nextSide = nextHeight >= 0.0 ? 1.0 : -1.0;
    bool crossedPlane = state.diskSide * nextSide < 0.0;
    float radialFeather = max(halfThickness * 1.35, 0.055);
    bool orderedCrossing = crossedPlane &&
        closestRadius <= outerRadius + radialFeather;
    if (crossedPlane)
    {
        state.diskSide = nextSide;
    }

    if (orderedCrossing)
    {
        state.diskPlaneCrossings += 1.0;
    }

    // Samples immediately before a crossing belong to the upcoming passage. This
    // keeps all volume samples from one physical traversal on the same image order.
    bool approachingPlane = !crossedPlane &&
        abs(nextHeight) < abs(previousHeight) &&
        closestRadius <= outerRadius + radialFeather &&
        abs(nextHeight) <= flaredThickness * 3.2;
    float diskPassage = max(
        state.diskPlaneCrossings + (approachingPlane ? 1.0 : 0.0),
        1.0);

    // Integrate a short window around the disk-plane closest approach. Composite
    // Simpson sampling keeps a thin, curved disk continuous even when the closest
    // approach falls between the regular ray steps.
    float halfWindowT = abs(heightDelta) > 0.00001
        ? min(1.0, flaredThickness * 2.8 / abs(heightDelta))
        : 0.5;
    float windowStart = max(0.0, closestT - halfWindowT);
    float windowEnd = min(1.0, closestT + halfWindowT);
    float windowSpan = windowEnd - windowStart;
    if (windowSpan <= 0.0001 || state.transmittance <= 0.002)
    {
        return;
    }

    float t0 = windowStart;
    float t1 = lerp(windowStart, windowEnd, 0.25);
    float t2 = lerp(windowStart, windowEnd, 0.50);
    float t3 = lerp(windowStart, windowEnd, 0.75);
    float t4 = windowEnd;
    float3 p0 = previousPosition + segment * t0;
    float3 p1 = previousPosition + segment * t1;
    float3 p2 = previousPosition + segment * t2;
    float3 p3 = previousPosition + segment * t3;
    float3 p4 = previousPosition + segment * t4;
    float d0 = SunkEvaluateDiskDensity(p0, diskNormal, innerRadius, outerRadius, halfThickness);
    float d1 = SunkEvaluateDiskDensity(p1, diskNormal, innerRadius, outerRadius, halfThickness);
    float d2 = SunkEvaluateDiskDensity(p2, diskNormal, innerRadius, outerRadius, halfThickness);
    float d3 = SunkEvaluateDiskDensity(p3, diskNormal, innerRadius, outerRadius, halfThickness);
    float d4 = SunkEvaluateDiskDensity(p4, diskNormal, innerRadius, outerRadius, halfThickness);
    float weightedDensity = d0 + 4.0 * d1 + 2.0 * d2 + 4.0 * d3 + d4;
    if (weightedDensity <= 0.0005)
    {
        return;
    }

    float3 samplePosition =
        (p0 * d0 + p1 * (4.0 * d1) + p2 * (2.0 * d2) +
         p3 * (4.0 * d3) + p4 * d4) /
        max(weightedDensity, 0.000001);
    float height = dot(samplePosition, diskNormal);
    float3 radialVector = samplePosition - diskNormal * height;
    float diskRadius = length(radialVector);
    radial01 = saturate((diskRadius - innerRadius) / max(outerRadius - innerRadius, 0.001));
    flaredThickness = halfThickness * lerp(0.82, 1.38, radial01);
    float integratedDensity = windowSpan * weightedDensity / 12.0;
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

    float crossingOrientation = dot(tracedDirection, diskNormal);
    alpha *= SunkDiskOrderAttenuation(
        state,
        diskPassage,
        diskRadius,
        crossingOrientation);
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
    state.diskPlaneCrossings = 0.0;
    state.diskSide = 0.0;
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

        float3 acceleration = SunkRayAcceleration(
            previousPosition,
            previousDirection,
            spinAxis,
            horizonRadius,
            massScale,
            lensingScale);

        float accelerationMagnitude = length(acceleration);
        float distanceStep = minimumStep + 0.14 * max(radius - horizonRadius, 0.0);
        float turningStep = maximumTurn / max(accelerationMagnitude, 0.0001);
        float stepLength = clamp(min(distanceStep, turningStep), minimumStep, maximumStep);
        float photonProximity = SunkGaussian(
            radius - 1.5 * horizonRadius,
            horizonRadius * 0.42);
        stepLength = max(minimumStep, lerp(stepLength, stepLength * 0.52, photonProximity));

        float3 midpointDirection = SunkSafeNormalize(
            previousDirection + acceleration * (stepLength * 0.5));
        float3 midpointPosition = previousPosition + midpointDirection * (stepLength * 0.5);
        float3 midpointAcceleration = SunkRayAcceleration(
            midpointPosition,
            midpointDirection,
            spinAxis,
            horizonRadius,
            massScale,
            lensingScale);
        float3 turn = midpointAcceleration * stepLength;
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
        float radialAlignment = abs(dot(
            previousDirection,
            previousPosition / max(radius, 0.0001)));
        float orbitalTangency = smoothstep(0.18, 0.82, 1.0 - radialAlignment);
        state.photonResidency +=
            SunkGaussian(radius - photonRadius, horizonRadius * 0.18) *
            lerp(0.24, 1.0, orbitalTangency) *
            stepLength / horizonRadius;

        SunkAccumulateDisk(
            state,
            previousPosition,
            nextPosition,
            previousDirection,
            nextDirection,
            stepLength,
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
