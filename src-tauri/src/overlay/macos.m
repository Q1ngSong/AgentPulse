#import <AppKit/AppKit.h>

// NSWorkspace 的激活事件也覆盖 Cmd+Tab、Dock、点击来源窗口等手动返回方式。
// 只监听切换事件，不因来源恰好在前台就吞掉之后产生的新提醒。
void ap_overlay_observe_activations(void (*callback)(const char *)) {
    static id observer;
    if (observer) return;
    observer = [NSWorkspace.sharedWorkspace.notificationCenter
        addObserverForName:NSWorkspaceDidActivateApplicationNotification
        object:nil queue:NSOperationQueue.mainQueue
        usingBlock:^(NSNotification *notification) {
            NSRunningApplication *application = notification.userInfo[NSWorkspaceApplicationKey];
            if (![application isKindOfClass:NSRunningApplication.class]) return;
            NSString *bundle = application.bundleIdentifier;
            if (bundle.length > 0) callback(bundle.UTF8String);
        }];
}

void ap_overlay_reflow_later(void (*callback)(void)) {
    dispatch_async(dispatch_get_main_queue(), ^{ callback(); });
}
