/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
 * SPDX-License-Identifier: MIT
 *
 * The application entry point. It loads the one page this scaffold ships; every
 * designer-specific behaviour lives in `pages/Index.ets` and the native bridge.
 */
import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';

const DOMAIN: number = 0x0000;
const TAG: string = 'rustage';

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    hilog.info(DOMAIN, TAG, 'entry ability created');
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index', (error: BusinessError) => {
      if (error.code !== 0) {
        hilog.error(DOMAIN, TAG, 'loading pages/Index failed: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(DOMAIN, TAG, 'pages/Index loaded');
    });
  }

  onWindowStageDestroy(): void {
    hilog.info(DOMAIN, TAG, 'window stage destroyed');
  }

  onDestroy(): void {
    hilog.info(DOMAIN, TAG, 'entry ability destroyed');
  }
}
