package com.handcontrol.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import com.handcontrol.feature.welcome.WelcomeScreen
import com.handcontrol.feature.welcome.WelcomeViewModel

@Composable
fun HandControlNavHost(
    modifier: Modifier = Modifier,
    navController: NavHostController = rememberNavController(),
    welcomeViewModel: WelcomeViewModel = hiltViewModel()
) {
    val lastConnectedServer by welcomeViewModel.lastConnectedServer
        .collectAsStateWithLifecycle()

    var hasAutoNavigated by remember { mutableStateOf(false) }
    NavHost(
        navController = navController,
        startDestination = Route.Welcome,
        modifier = modifier
    ) {
        composable<Route.Welcome> {
            WelcomeScreen(
                onNavigateToServerDiscovery = {
                    navController.navigate(Route.ServerDiscovery)
                },
                onNavigateToServerList = {
                    navController.navigate(Route.ServerList)
                }
            )
        }

        composable<Route.ServerDiscovery> {
            com.handcontrol.feature.discovery.ServerDiscoveryScreen(
                onNavigateToQrEnrollment = {
                    navController.navigate(Route.EnrollmentQr("", 0))
                },
                onNavigateToApprovalEnrollment = { host, port, serverId ->
                    navController.navigate(Route.EnrollmentApproval(host, port, serverId))
                }
            )
        }

        composable<Route.ServerList> {
            com.handcontrol.feature.serverlist.ServerListScreen(
                onNavigateBack = {
                    navController.popBackStack()
                },
                onNavigateToServerDiscovery = {
                    navController.navigate(Route.ServerDiscovery)
                },
                onNavigateToServer = { host, port ->
                    navController.navigate(Route.CommandList(host, port))
                }
            )
        }

        composable<Route.EnrollmentQr> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.EnrollmentQr>()
            com.handcontrol.feature.enrollment.QrScannerScreen(
                onNavigateBack = {
                    navController.popBackStack()
                },
                onQrCodeScanned = { host, port, token ->
                    navController.navigate(Route.CommandList(host, port)) {
                        popUpTo(Route.Welcome) { inclusive = true }
                    }
                }
            )
        }

        composable<Route.EnrollmentApproval> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.EnrollmentApproval>()
            com.handcontrol.feature.enrollment.ApprovalPairingScreen(
                serverHost = route.serverHost,
                serverPort = route.serverPort,
                serverId = route.serverId,
                onNavigateBack = {
                    navController.popBackStack()
                },
                onNavigateToCommands = { host, port ->
                    navController.navigate(Route.CommandList(host, port)) {
                        popUpTo(Route.Welcome) { inclusive = true }
                    }
                }
            )
        }

        composable<Route.CommandList> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandList>()
            com.handcontrol.feature.commands.CommandListScreen(
                serverHost = route.serverHost,
                serverPort = route.serverPort,
                onNavigateToCommandExecution = { host, port, commandId ->
                    navController.navigate(Route.CommandExecution(host, port, commandId))
                },
                onNavigateBack = {
                    navController.popBackStack()
                },
                onNavigateToServerList = {
                    navController.navigate(Route.ServerList)
                }
            )
        }

        composable<Route.CommandDetail> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandDetail>()
            // Command detail screen - to be implemented
        }

        composable<Route.CommandExecution> { backStackEntry ->
            val route = backStackEntry.toRoute<Route.CommandExecution>()
            com.handcontrol.feature.commands.CommandExecutionScreen(
                serverHost = route.serverHost,
                serverPort = route.serverPort,
                commandId = route.commandId,
                onNavigateBack = {
                    navController.popBackStack()
                }
            )
        }
    }

    LaunchedEffect(lastConnectedServer?.serverId) {
        if (hasAutoNavigated) return@LaunchedEffect
        val server = lastConnectedServer ?: return@LaunchedEffect

        navController.navigate(
            Route.CommandList(server.serverHost, server.serverPort)
        ) {
            popUpTo(Route.Welcome) { inclusive = false }
        }
        hasAutoNavigated = true
    }
}
