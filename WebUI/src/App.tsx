/*
 * Copyright (C) Contributors to the Suwayomi project
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import Box from '@mui/material/Box';
import CssBaseline from '@mui/material/CssBaseline';
import React, { useEffect, useLayoutEffect } from 'react';
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { AwaitableComponent } from 'awaitable-component';
import { loadable } from 'react-lazily/loadable';
import { AppContext } from '@/base/contexts/AppContext';
import { AppRoutes } from '@/base/AppRoute.constants';
import { ErrorBoundary } from '@/base/components/feedback/ErrorBoundary';
import { lazyLoadFallback } from '@/base/utils/LazyLoad';
import { DefaultNavBar } from '@/features/navigation-bar/components/DefaultNavBar';
import { useNavBarContext } from '@/features/navigation-bar/NavbarContext';
import { ReactRouter } from '@/lib/react-router/ReactRouter';
import { requestManager } from '@/lib/requests/RequestManager';
import { AuthManager } from '@/features/authentication/AuthManager';
import { OCRProvider } from '@/Manatan/context/OCRContext';
import { OCRManager } from '@/Manatan/OCRManager';
import { Activity } from '@/features/fork/Activity';
import { Stats } from '@/features/fork/Stats';
import { SourceOnboarding } from '@/features/fork/SourceOnboarding';
import { SeriesDetail } from '@/features/fork/SeriesDetail';
import { BackgroundJobBridge } from '@/features/fork/BackgroundJobBridge';

const { Library } = loadable(() => import('@/features/library/screens/Library'), lazyLoadFallback);
const { Manga } = loadable(() => import('@/features/manga/screens/Manga'), lazyLoadFallback);
const { Reader } = loadable(() => import('@/features/reader/screens/Reader'), lazyLoadFallback);
const { Settings } = loadable(() => import('@/features/settings/screens/Settings'), lazyLoadFallback);
const { Dictionary } = loadable(() => import('@/features/dictionary/Dictionary'), lazyLoadFallback);

// This fork's loopback server is always local and account-free. Initialize the
// inherited request queue explicitly without mounting Manatan's login guard.
if (!AuthManager.isAuthInitialized()) {
    AuthManager.setAuthRequired(false);
    AuthManager.setAuthInitialized(true);
    requestManager.processQueues();
}

const ScrollToTop = () => {
    const { pathname } = useLocation();
    useLayoutEffect(() => {
        window.scrollTo(0, 0);
    }, [pathname]);
    return null;
};

const ReactRouterSetter = () => {
    const navigate = useNavigate();
    useEffect(() => {
        ReactRouter.setNavigateFn(navigate);
    }, [navigate]);
    return null;
};

const MainApp = () => {
    const { navBarWidth, appBarHeight, bottomBarHeight } = useNavBarContext();
    return (
        <Box
            id="appMainContainer"
            component="main"
            sx={{
                minHeight: `calc(100vh - ${appBarHeight + bottomBarHeight}px)`,
                width: `calc(100vw - (100vw - 100%) - ${navBarWidth}px)`,
                minWidth: 0,
                position: 'relative',
                mt: `${appBarHeight}px`,
                pb: `calc(${bottomBarHeight}px + ${!bottomBarHeight ? 'env(safe-area-inset-bottom)' : '0px'})`,
                pl: 'env(safe-area-inset-left)',
                pr: 'env(safe-area-inset-right)',
            }}
        >
            <ErrorBoundary>
                <Routes>
                    <Route path={AppRoutes.root.match} element={<Navigate to={AppRoutes.library.path()} replace />} />
                    <Route path={AppRoutes.library.match} element={<Library />} />
                    <Route path="/fork/series/:seriesId" element={<SeriesDetail />} />
                    <Route path={AppRoutes.activity.match} element={<Activity />} />
                    <Route path={AppRoutes.stats.match} element={<Stats />} />
                    <Route path={AppRoutes.settings.match} element={<Settings />} />
                    <Route path={AppRoutes.dictionary.match} element={<Dictionary />} />
                    <Route path={AppRoutes.manga.match}>
                        <Route path={AppRoutes.manga.childRoutes.reader.match} element={null} />
                        <Route index element={<Manga />} />
                    </Route>
                    <Route
                        path={AppRoutes.matchAll.match}
                        element={<Navigate to={AppRoutes.library.path()} replace />}
                    />
                </Routes>
            </ErrorBoundary>
        </Box>
    );
};

const ReaderApp = () => (
    <ErrorBoundary>
        <Routes>
            <Route path={AppRoutes.matchAll.match} element={<Reader />} />
        </Routes>
    </ErrorBoundary>
);

export const App: React.FC = () => (
    <AppContext>
        <ScrollToTop />
        <AwaitableComponent.Root />
        <ReactRouterSetter />
        <CssBaseline enableColorScheme />
        <OCRProvider>
            <SourceOnboarding />
            <BackgroundJobBridge />
            <OCRManager />
            <Routes>
                <Route path="/fork/reader/:volumeId" element={<Reader forkMode />} />
                <Route path={AppRoutes.reader.match} element={<ReaderApp />} />
                <Route
                    path="*"
                    element={
                        <Box sx={{ display: 'flex' }}>
                            <Box sx={{ flexShrink: 0, position: 'relative', height: '100vh' }}>
                                <DefaultNavBar />
                            </Box>
                            <MainApp />
                        </Box>
                    }
                />
            </Routes>
        </OCRProvider>
    </AppContext>
);
