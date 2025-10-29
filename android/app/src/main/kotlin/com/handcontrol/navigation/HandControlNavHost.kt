package com.handcontrol.navigation

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import com.handcontrol.feature.welcome.WelcomeScreen

@Composable
fun HandControlNavHost(
    modifier: Modifier = Modifier,
    navController: NavHostController = rememberNavController()
) {
    NavHost(
        navController = navController,
        startDestination = Route.Welcome,
        modifier = modifier
    ) {
        composable<Route.Welcome> {
            WelcomeScreen(
                onNavigateToServerDiscovery = {
                    navController.navigate(Route.ServerDiscovery)
                }
            )
        }

        composable<Route.ServerDiscovery> {
            com.handcontrol.feature.discovery.ServerDiscoveryScreen(
                onNavigateToQrEnrollment = {
                    // TODO: Navigate to QR enrollment
                },
                onNavigateToApprovalEnrollment = { host, port ->
                    navController.navigate(Route.EnrollmentApproval(host, port))
                }
            )
        }

        composable<Route.EnrollmentQr> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.EnrollmentQr>()
            // QR enrollment screen - to be implemented
        }

        composable<Route.EnrollmentApproval> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.EnrollmentApproval>()
            // Approval enrollment screen - to be implemented
        }

        composable<Route.CommandList> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandList>()
            // Command list screen - to be implemented
        }

        composable<Route.CommandDetail> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandDetail>()
            // Command detail screen - to be implemented
        }

        composable<Route.CommandExecution> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandExecution>()
            // Command execution screen - to be implemented
        }
    }
}
