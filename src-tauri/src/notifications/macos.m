// UserNotifications 的薄桥接层，由 Rust 主程序静态链接，不另起通知进程。
#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>

typedef void (*APResponseCallback)(const char *, const char *, const char *);
typedef void (*APLaunchCallback)(int);
typedef void (*APCompletion)(void *, const char *);
typedef void (*APDefaultClickCallback)(void);

static NSString *const APHostKey = @"agentpulse.host";
static NSString *const APSourceKey = @"agentpulse.source_host";
static NSString *const APReturnAction = @"agentpulse.return";
static NSString *const APReturnCategory = @"agentpulse.source";
static const int64_t APNotificationLifetime = 5 * NSEC_PER_SEC;
static APResponseCallback responseCallback;
static APLaunchCallback launchCallback;
static APDefaultClickCallback defaultClickCallback;
static UNUserNotificationCenter *center;

static BOOL hasManagedSource(UNNotificationResponse *response) {
    return [response.notification.request.content.userInfo[APHostKey] isKindOfClass:NSString.class];
}

static void markDefaultClick(UNNotificationResponse *response) {
    if (hasManagedSource(response) &&
        [response.actionIdentifier isEqualToString:UNNotificationDefaultActionIdentifier]) {
        // 必须在转交主队列和 completion 之前标记，否则 Reopen 可能先打开面板。
        defaultClickCallback();
    }
}

static void handleResponse(UNNotificationResponse *response) {
    if (!hasManagedSource(response)) return; // 旧版通知保持普通打开应用的行为。
    NSString *host = response.notification.request.content.userInfo[APHostKey];
    if (![host isKindOfClass:NSString.class]) host = @"";
    NSString *action = @"other";
    if ([response.actionIdentifier isEqualToString:UNNotificationDefaultActionIdentifier]) action = @"default";
    else if ([response.actionIdentifier isEqualToString:APReturnAction]) action = @"return";
    else if ([response.actionIdentifier isEqualToString:UNNotificationDismissActionIdentifier]) action = @"dismiss";
    responseCallback(response.notification.request.identifier.UTF8String, action.UTF8String, host.UTF8String);
}

@interface APNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@end

@implementation APNotificationDelegate
- (void)userNotificationCenter:(UNUserNotificationCenter *)sender
      willPresentNotification:(UNNotification *)notification
        withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completion {
    // 前台也显示系统横幅；声音仍由 AgentPulse 的独立声音渠道控制。
    completion(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList);
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)sender
didReceiveNotificationResponse:(UNNotificationResponse *)response
        withCompletionHandler:(void (^)(void))completion {
    markDefaultClick(response);
    dispatch_async(dispatch_get_main_queue(), ^{
        completion();
        // 先完成通知响应，再处理来源跳转，避免完成通知时的激活覆盖目标 App 的焦点。
        dispatch_async(dispatch_get_main_queue(), ^{ handleResponse(response); });
    });
}

- (void)didFinishLaunching:(NSNotification *)notification {
    // macOS 将冷启动响应放在这个键下，现代通知的值为 UNNotificationResponse。
    // 注册观察者，不替换 Tauri/tao 的 NSApplicationDelegate。
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    id response = notification.userInfo[NSApplicationLaunchUserNotificationKey];
#pragma clang diagnostic pop
    if ([response isKindOfClass:UNNotificationResponse.class]) markDefaultClick(response);
    // 下一轮主队列再处理，确保 AppKit 的所有启动观察者及 Tauri setup 已完成。
    dispatch_async(dispatch_get_main_queue(), ^{
        BOOL modernResponse = [response isKindOfClass:UNNotificationResponse.class];
        BOOL hasSource = modernResponse &&
            [((UNNotificationResponse *)response).notification.request.content.userInfo[APHostKey]
                isKindOfClass:NSString.class];
        // 旧版通知没有来源字段，保持普通打开面板的行为；空字符串则是明确选择“只关闭”。
        launchCallback(hasSource);
        if (modernResponse) handleResponse(response);
    });
}
@end

// UNUserNotificationCenter.delegate 是 weak，必须持有到进程结束。
static APNotificationDelegate *delegate;

void ap_notifications_init(APResponseCallback response, APLaunchCallback launch, APDefaultClickCallback defaultClick) {
    responseCallback = response;
    launchCallback = launch;
    defaultClickCallback = defaultClick;
    delegate = [APNotificationDelegate new];
    [NSNotificationCenter.defaultCenter addObserver:delegate
        selector:@selector(didFinishLaunching:) name:NSApplicationDidFinishLaunchingNotification object:nil];
    // 裸 cargo/tauri dev 二进制没有应用 bundle，调用 currentNotificationCenter 会抛异常。
    if (!NSBundle.mainBundle.bundleIdentifier.length ||
        ![NSBundle.mainBundle.bundleURL.pathExtension isEqualToString:@"app"]) return;
    center = UNUserNotificationCenter.currentNotificationCenter;
    center.delegate = delegate;
    UNNotificationAction *action = [UNNotificationAction actionWithIdentifier:APReturnAction
        title:@"返回来源" options:UNNotificationActionOptionNone];
    UNNotificationCategory *category = [UNNotificationCategory categoryWithIdentifier:APReturnCategory
        actions:@[action] intentIdentifiers:@[] options:UNNotificationCategoryOptionCustomDismissAction];
    [center setNotificationCategories:[NSSet setWithObject:category]];
}

// 查询由系统保存的已送达记录，重启后也能清理；只移除这次聚焦前的匹配 ID。
void ap_notifications_dismiss_source(const char *source, double before) {
    @autoreleasepool {
        if (!center || !source || !source[0]) return;
        NSString *bundle = [NSString stringWithUTF8String:source];
        NSDate *cutoff = [NSDate dateWithTimeIntervalSince1970:before];
        [center getDeliveredNotificationsWithCompletionHandler:^(NSArray<UNNotification *> *notifications) {
            NSMutableArray<NSString *> *identifiers = [NSMutableArray array];
            for (UNNotification *notification in notifications) {
                NSDictionary *info = notification.request.content.userInfo;
                // 已经送达的旧通知尚无独立来源字段；明确的空来源不能回退成点击目标。
                id host = info[APSourceKey] ?: info[APHostKey];
                if ([host isKindOfClass:NSString.class] && [host isEqualToString:bundle] &&
                    notification.date && [notification.date compare:cutoff] != NSOrderedDescending) {
                    [identifiers addObject:notification.request.identifier];
                }
            }
            if (identifiers.count) [center removeDeliveredNotificationsWithIdentifiers:identifiers];
        }];
    }
}

void ap_notifications_send(const char *identifier, const char *title, const char *body,
                           const char *host, const char *source_host, void *context, APCompletion completion) {
    @autoreleasepool {
        // Rust 的 CString 只活到本函数返回；异步调用前必须复制到 ARC 持有的对象。
        NSString *requestID = [NSString stringWithUTF8String:identifier];
        UNMutableNotificationContent *content = [UNMutableNotificationContent new];
        content.title = [NSString stringWithUTF8String:title];
        content.body = [NSString stringWithUTF8String:body];
        NSString *bundle = [NSString stringWithUTF8String:host];
        // 来源用于聚焦清理，点击目标为空仍表示“只关闭”。
        content.userInfo = @{APHostKey: bundle, APSourceKey: [NSString stringWithUTF8String:source_host]};
        if (bundle.length) content.categoryIdentifier = APReturnCategory;
        // center 在进程级持有；回调用弱引用避免通知中心与 completion 相互持有。
        __weak UNUserNotificationCenter *notificationCenter = center;
        if (!notificationCenter) {
            completion(context, "系统通知需要从打包后的 AgentPulse.app 运行；开发模式可使用悬浮窗");
            return;
        }
        // UserNotifications 支持后台调用；不能等主队列，面板的同步“测试”命令正在等这个 IPC 回复。
        [notificationCenter requestAuthorizationWithOptions:UNAuthorizationOptionAlert
            completionHandler:^(BOOL granted, NSError *error) {
                if (!granted || error) {
                    NSString *message = error.localizedDescription ?: @"系统通知权限未开启，请在 macOS 系统设置 → 通知 → AgentPulse 中允许通知";
                    completion(context, message.UTF8String);
                    return;
                }
                UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:requestID
                    content:content trigger:nil];
                [notificationCenter addNotificationRequest:request withCompletionHandler:^(NSError *error) {
                    if (!error) {
                        // 系统接受请求后开始计时；只清理这条唯一 ID，稍后到达的新通知有自己的计时器。
                        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, APNotificationLifetime), dispatch_get_main_queue(), ^{
                            [notificationCenter removePendingNotificationRequestsWithIdentifiers:@[requestID]];
                            [notificationCenter removeDeliveredNotificationsWithIdentifiers:@[requestID]];
                        });
                    }
                    completion(context, error ? error.localizedDescription.UTF8String : NULL);
                }];
            }];
    }
}
