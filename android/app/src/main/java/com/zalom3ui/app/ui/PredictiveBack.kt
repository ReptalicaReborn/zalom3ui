package com.zalom3ui.app.ui

import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.EnterExitState
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.AbsoluteRoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavController

/*
 * Back (the predictive back gesture, the system back button and the app's back arrows) slides the current screen off
 * to the right and reveals the previous one underneath.
 *
 * During the gesture NavHost seeks its transition to the gesture's progress: playTime = progress × duration. With
 * linear easing the screen is therefore exactly `progress × width` from the left edge, i.e. it stays under the finger.
 * When the finger lifts, the rest of the slide plays at the same speed; a cancelled gesture eases back into place.
 */

/** Length of the back slide. Also how long the remaining slide takes when the finger lifts at the very start. */
internal const val BACK_SLIDE_MS = 280

/** How far left the previous screen starts (a fraction of the width), so it glides in a little behind the top one. */
private const val PARALLAX_DIVISOR = 5

/** Darkening over the revealed screen when the gesture starts; it clears as the top screen leaves. */
private const val MAX_SCRIM_ALPHA = 0.18f

private val CornerRadius = 24.dp
private val EdgeShadow = 12.dp

private fun <T> backSlideSpec() = tween<T>(durationMillis = BACK_SLIDE_MS, easing = LinearEasing)

/** The screen being popped: follows the gesture 1:1, then slides off the right edge. */
fun backSlideOut(): ExitTransition = slideOutHorizontally(backSlideSpec<IntOffset>()) { fullWidth -> fullWidth }

/** The screen revealed by back: glides in from slightly left (parallax) at the same pace. */
fun backSlideIn(): EnterTransition =
    slideInHorizontally(backSlideSpec<IntOffset>()) { fullWidth -> -fullWidth / PARALLAX_DIVISOR }

/**
 * Whether a screen that is leaving is being popped (back) rather than covered by a newly opened screen.
 * While the back gesture is in progress the screen is still the top of the back stack; once back is committed (or
 * after a back arrow / button press) it is no longer on the back stack at all. A covered screen is neither.
 */
internal fun leavingByBack(isTopEntry: Boolean, stillOnBackStack: Boolean): Boolean = isTopEntry || !stillOnBackStack

/** Darkening drawn over the revealed screen at [progress] (0 = gesture start, 1 = screen gone). */
internal fun backScrimAlpha(progress: Float): Float = MAX_SCRIM_ALPHA * (1f - progress.coerceIn(0f, 1f))

/** How much of [CornerRadius] the departing screen's left corners have: rounded within the first few % of the drag. */
internal fun backCornerFraction(progress: Float): Float = (progress * 12f).coerceIn(0f, 1f)

/**
 * Wraps a navigation destination so that, while it is popped, it looks like a card being pulled away: rounded left
 * corners, a shadow along its left edge, and a scrim over the screen it reveals that lightens as it leaves.
 * Nothing changes while the screen is at rest or when another screen opens on top of it.
 */
@Composable
fun AnimatedVisibilityScope.BackSlideFrame(
    nav: NavController,
    entry: NavBackStackEntry,
    content: @Composable () -> Unit,
) {
    val exiting = transition.targetState == EnterExitState.PostExit
    // Decided once when the screen starts leaving, so it holds through the gesture, the commit and a cancel.
    val byBack = remember(exiting) {
        exiting && leavingByBack(
            isTopEntry = nav.currentBackStackEntry?.id == entry.id,
            stillOnBackStack = runCatching { nav.getBackStackEntry(entry.destination.id).id == entry.id }.getOrDefault(false),
        )
    }
    // Same linear timing as the slide, so this is exactly how far the screen has moved (0..1 of its width).
    val progress by transition.animateFloat(
        transitionSpec = { if (byBack) backSlideSpec() else snap() },
        label = "backSlide",
    ) { state -> if (state == EnterExitState.PostExit) 1f else 0f }

    Box(
        Modifier
            .fillMaxSize()
            // The scrim sits just left of this screen, i.e. over the part of the previous screen it has uncovered.
            // At rest that area is off-screen, so it costs nothing to keep.
            .drawBehind {
                if (byBack) {
                    drawRect(
                        color = Color.Black.copy(alpha = backScrimAlpha(progress)),
                        topLeft = Offset(-size.width, 0f),
                        size = size,
                    )
                }
            }
            .graphicsLayer {
                if (byBack) {
                    val radius = CornerRadius.toPx() * backCornerFraction(progress)
                    shape = AbsoluteRoundedCornerShape(topLeft = radius, bottomLeft = radius)
                    clip = true
                    shadowElevation = EdgeShadow.toPx()
                }
            },
    ) {
        content()
    }
}
