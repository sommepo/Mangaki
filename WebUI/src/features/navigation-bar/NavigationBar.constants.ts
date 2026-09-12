/*
 * Copyright (C) Contributors to the Suwayomi project
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import MenuBookIcon from '@mui/icons-material/MenuBook';
import MenuBookOutlinedIcon from '@mui/icons-material/MenuBookOutlined';
import BarChartIcon from '@mui/icons-material/BarChart';
import BarChartOutlinedIcon from '@mui/icons-material/BarChartOutlined';
import SettingsIcon from '@mui/icons-material/Settings';
import SettingsOutlinedIcon from '@mui/icons-material/SettingsOutlined';
import type { NavbarItem } from '@/features/navigation-bar/NavigationBar.types.ts';
import { NavBarItemMoreGroup } from '@/features/navigation-bar/NavigationBar.types.ts';
import { AppRoutes } from '@/base/AppRoute.constants.ts';

export const NAVIGATION_BAR_ITEMS = [
    {
        path: AppRoutes.library.path(),
        title: 'Library' as NavbarItem['title'],
        SelectedIconComponent: MenuBookIcon,
        IconComponent: MenuBookOutlinedIcon,
        show: 'both',
        moreGroup: NavBarItemMoreGroup.GENERAL,
    },
    {
        path: AppRoutes.stats.path,
        title: 'Stats' as NavbarItem['title'],
        SelectedIconComponent: BarChartIcon,
        IconComponent: BarChartOutlinedIcon,
        show: 'both',
        moreGroup: NavBarItemMoreGroup.GENERAL,
    },
    {
        path: AppRoutes.settings.path,
        title: 'Settings' as NavbarItem['title'],
        SelectedIconComponent: SettingsIcon,
        IconComponent: SettingsOutlinedIcon,
        show: 'both',
        moreGroup: NavBarItemMoreGroup.SETTING_INFO,
    },
] as const satisfies NavbarItem[];

export const NAVIGATION_BAR_MOBILE_ITEMS = NAVIGATION_BAR_ITEMS;
