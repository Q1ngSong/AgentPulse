// clang -fobjc-arc -fblocks -framework AppKit -framework UserNotifications -framework CoreFoundation \
//   src-tauri/tests/notifications.m -o /tmp/agentpulse-notifications-test && /tmp/agentpulse-notifications-test
#import <CoreFoundation/CoreFoundation.h>
#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>

static dispatch_time_t testDispatchTime(dispatch_time_t when, int64_t delta);
static void testDispatchAfter(dispatch_time_t when, dispatch_queue_t queue, dispatch_block_t block);
#define dispatch_time testDispatchTime
#define dispatch_after testDispatchAfter
#import "../src/notifications/macos.m"
#undef dispatch_time
#undef dispatch_after

@interface TestTimer : NSObject
@property(nonatomic) dispatch_time_t deadline;
@property(nonatomic, copy) dispatch_block_t block;
@end
@implementation TestTimer
@end
static NSMutableArray<TestTimer *> *timers;
static dispatch_time_t now;
static dispatch_time_t testDispatchTime(dispatch_time_t when, int64_t delta) {
    NSCAssert(when == DISPATCH_TIME_NOW && delta == 5 * NSEC_PER_SEC, @"notification must expire in five seconds");
    return now + delta;
}
static void testDispatchAfter(dispatch_time_t when, dispatch_queue_t queue, dispatch_block_t block) {
    NSCAssert(queue == dispatch_get_main_queue(), @"expiration must use the main queue");
    TestTimer *timer = [TestTimer new];
    timer.deadline = when;
    timer.block = block;
    [timers addObject:timer];
}
static void advanceTo(dispatch_time_t time) {
    now = time;
    for (TestTimer *timer in [timers copy]) {
        if (timer.deadline <= now) {
            [timers removeObject:timer];
            timer.block();
        }
    }
}

@interface TestNotification : NSObject
@property(nonatomic, strong) UNNotificationRequest *request;
@property(nonatomic, strong) NSDate *date;
@end
@implementation TestNotification
@end
@interface TestResponse : NSObject
@property(nonatomic, strong) TestNotification *notification;
@property(nonatomic, copy) NSString *actionIdentifier;
@end
@implementation TestResponse
@end

// All notification-center calls end here; the tests never contact the real service.
@interface TestCenter : NSObject
@property(nonatomic, strong) NSMutableArray *delivered;
@property(nonatomic, copy) void (^query)(NSArray<UNNotification *> *);
@property(nonatomic, copy) NSArray<NSString *> *removed;
@property(nonatomic, copy) NSArray<NSString *> *removedPending;
@property(nonatomic, strong) UNNotificationRequest *sent;
@property(nonatomic) BOOL denyAuthorization;
@property(nonatomic) BOOL deferAddCompletion;
@property(nonatomic, strong) NSError *addError;
@property(nonatomic, copy) void (^addCompletion)(NSError *);
@end
@implementation TestCenter
- (void)getDeliveredNotificationsWithCompletionHandler:(void (^)(NSArray<UNNotification *> *))completion {
    NSCAssert(!self.query, @"overlapping query");
    self.query = completion;
}
- (void)answerQuery {
    void (^query)(NSArray *) = self.query;
    self.query = nil;
    if (query) query([self.delivered copy]);
}
- (void)removeDeliveredNotificationsWithIdentifiers:(NSArray<NSString *> *)identifiers {
    self.removed = identifiers;
}
- (void)removePendingNotificationRequestsWithIdentifiers:(NSArray<NSString *> *)identifiers {
    self.removedPending = identifiers;
}
- (void)applyRemoval {
    NSIndexSet *ids = [self.delivered indexesOfObjectsPassingTest:^BOOL(TestNotification *n, NSUInteger i, BOOL *stop) {
        return [self.removed containsObject:n.request.identifier];
    }];
    [self.delivered removeObjectsAtIndexes:ids];
}
- (void)requestAuthorizationWithOptions:(UNAuthorizationOptions)options completionHandler:(void (^)(BOOL, NSError *))completion {
    completion(!self.denyAuthorization, nil);
}
- (void)addNotificationRequest:(UNNotificationRequest *)request withCompletionHandler:(void (^)(NSError *))completion {
    self.sent = request;
    if (self.deferAddCompletion) self.addCompletion = completion;
    else completion(self.addError);
}
@end

static TestNotification *notice(NSString *identifier, NSDictionary *info, double date) {
    UNMutableNotificationContent *content = [UNMutableNotificationContent new];
    content.userInfo = info;
    TestNotification *n = [TestNotification new];
    n.request = [UNNotificationRequest requestWithIdentifier:identifier content:content trigger:nil];
    n.date = [NSDate dateWithTimeIntervalSince1970:date];
    return n;
}

static NSMutableArray<NSString *> *events;
static NSString *expectedHost;
static void marked(void) { [events addObject:@"mark"]; }
static void responded(const char *identifier, const char *action, const char *host) {
    NSCAssert(strcmp(identifier, "focus-test") == 0, @"request ID changed");
    NSCAssert([[NSString stringWithUTF8String:host] isEqual:expectedHost], @"click target changed");
    [events addObject:[@"route:" stringByAppendingString:[NSString stringWithUTF8String:action]]];
}
static void checkClick(NSString *action, NSDictionary *info, NSArray *immediate, NSArray *eventual) {
    events = [NSMutableArray array];
    expectedHost = info[APHostKey];
    TestResponse *response = [TestResponse new];
    response.notification = notice(@"focus-test", info, 100);
    response.actionIdentifier = action;
    [[APNotificationDelegate new] userNotificationCenter:center didReceiveNotificationResponse:(UNNotificationResponse *)response
        withCompletionHandler:^{ [events addObject:@"complete"]; }];
    NSCAssert([events isEqual:immediate], @"guard not synchronous: %@", events);
    for (int n = 0; n < 30 && events.count < eventual.count; n++) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    NSCAssert([events isEqual:eventual], @"wrong callback ordering: %@ expected %@", events, eventual);
}

static void sent(void *context, const char *error) {
    NSCAssert(!error, @"send failed: %s", error);
    (*(int *)context)++;
}

static void failed(void *context, const char *error) {
    NSCAssert(error, @"expected a send error");
    (*(int *)context)++;
}

static void checkExpiration(void) {
    TestCenter *fake = [TestCenter new];
    center = (UNUserNotificationCenter *)fake;
    fake.delivered = [NSMutableArray array];
    timers = [NSMutableArray array];
    now = 0;
    int completions = 0;
    fake.deferAddCompletion = YES;
    ap_notifications_send("first", "Title", "Body", "a", "a", &completions, sent);
    NSCAssert(!timers.count && completions == 0, @"timer started before successful submission");
    advanceTo(2 * NSEC_PER_SEC);
    fake.addCompletion(nil);
    fake.addCompletion = nil;
    fake.deferAddCompletion = NO;
    [fake.delivered addObject:notice(@"first", @{APSourceKey:@"a"}, 2)];
    NSCAssert(timers.count == 1 && completions == 1, @"successful submission needs exactly one timer");

    advanceTo(4 * NSEC_PER_SEC);
    ap_notifications_send("second", "Title", "Body", "a", "a", &completions, sent);
    [fake.delivered addObject:notice(@"second", @{APSourceKey:@"a"}, 4)];
    [fake.delivered addObject:notice(@"unrelated", @{APSourceKey:@"b"}, 4)];
    advanceTo(7 * NSEC_PER_SEC - 1);
    NSCAssert(!fake.removed && !fake.removedPending, @"notification expired too early");
    advanceTo(7 * NSEC_PER_SEC);
    NSCAssert([fake.removed isEqual:@[@"first"]] && [fake.removedPending isEqual:@[@"first"]], @"old timer removed another request");
    [fake applyRemoval];
    NSCAssert(([[fake.delivered valueForKeyPath:@"request.identifier"] isEqual:@[@"second", @"unrelated"]]), @"new or unrelated notification disappeared");
    NSCAssert(timers.count == 1, @"second timer was removed with the first");

    // Simulate a click or focus cleanup before expiration: the pending timer remains harmless.
    fake.removed = @[@"second"];
    [fake applyRemoval];
    advanceTo(9 * NSEC_PER_SEC);
    NSCAssert([fake.removed isEqual:@[@"second"]] && [fake.removedPending isEqual:@[@"second"]], @"each notification needs its own expiration");
    [fake applyRemoval];
    NSCAssert([[fake.delivered valueForKeyPath:@"request.identifier"] isEqual:@[@"unrelated"]], @"repeat removal affected another notification");
    NSCAssert(!timers.count && completions == 2, @"expiration completed sends again or left a timer");

    int failures = 0;
    fake.denyAuthorization = YES;
    ap_notifications_send("denied", "Title", "Body", "a", "a", &failures, failed);
    fake.denyAuthorization = NO;
    fake.addError = [NSError errorWithDomain:@"test" code:1 userInfo:nil];
    ap_notifications_send("rejected", "Title", "Body", "a", "a", &failures, failed);
    NSCAssert(failures == 2 && !timers.count, @"failed submissions must not schedule expiration");
}

int main(void) {
    @autoreleasepool {
        timers = [NSMutableArray array];
        TestCenter *fake = [TestCenter new];
        center = (UNUserNotificationCenter *)fake;
        fake.delivered = [@[
            notice(@"a1", @{APSourceKey:@"a", APHostKey:@"a"}, 99),
            notice(@"b", @{APSourceKey:@"b", APHostKey:@"b"}, 99),
            notice(@"close", @{APSourceKey:@"a", APHostKey:@""}, 99),
            notice(@"legacy", @{APHostKey:@"a"}, 99),
            notice(@"unknown", @{}, 99),
            notice(@"empty", @{APSourceKey:@"", APHostKey:@"a"}, 99),
            notice(@"source-wins", @{APSourceKey:@"b", APHostKey:@"a"}, 99),
            notice(@"cutoff", @{APSourceKey:@"a"}, 100),
            notice(@"new", @{APSourceKey:@"a"}, 101)
        ] mutableCopy];
        ap_notifications_dismiss_source("a", 100);
        NSCAssert(fake.query && !fake.removed, @"query must finish before removal");
        [fake.delivered addObject:notice(@"during-query", @{APSourceKey:@"a"}, 102)];
        [fake answerQuery];
        NSArray *expected = @[@"a1", @"close", @"legacy", @"cutoff"];
        NSCAssert([fake.removed isEqual:expected], @"wrong source or time selection: %@", fake.removed);
        [fake.delivered addObject:notice(@"after-query", @{APSourceKey:@"a"}, 103)];
        [fake applyRemoval];
        expected = @[@"b", @"unknown", @"empty", @"source-wins", @"new", @"during-query", @"after-query"];
        NSCAssert([[fake.delivered valueForKeyPath:@"request.identifier"] isEqual:expected], @"removed a new or unrelated notification");
        for (const char **source = (const char *[]){"", "missing", NULL}; *source; source++) {
            fake.removed = nil;
            ap_notifications_dismiss_source(*source, 200);
            [fake answerQuery];
            NSCAssert(!fake.removed.count, @"unknown source erased notifications");
        }

        int completions = 0;
        ap_notifications_send("close-send", "Title", "Body", "", "a", &completions, sent);
        NSCAssert(completions == 1, @"missing send completion");
        NSCAssert([fake.sent.content.userInfo[APHostKey] isEqual:@""], @"close-only acquired a target");
        NSCAssert([fake.sent.content.userInfo[APSourceKey] isEqual:@"a"], @"close-only lost its source");
        NSCAssert(!fake.sent.content.categoryIdentifier.length, @"close-only acquired a return action");
        ap_notifications_send("return-send", "Title", "Body", "target", "source", &completions, sent);
        NSCAssert(completions == 2, @"missing send completion");
        NSCAssert([fake.sent.content.userInfo[APHostKey] isEqual:@"target"], @"wrong click target");
        NSCAssert([fake.sent.content.userInfo[APSourceKey] isEqual:@"source"], @"wrong cleanup source");
        NSCAssert([fake.sent.content.categoryIdentifier isEqual:APReturnCategory], @"missing return action");

        defaultClickCallback = marked;
        responseCallback = responded;
        checkClick(UNNotificationDefaultActionIdentifier, @{APHostKey:@"a"}, @[@"mark"], @[@"mark", @"complete", @"route:default"]);
        checkClick(UNNotificationDefaultActionIdentifier, @{APHostKey:@"", APSourceKey:@"a"}, @[@"mark"], @[@"mark", @"complete", @"route:default"]);
        checkClick(APReturnAction, @{APHostKey:@"a"}, @[], @[@"complete", @"route:return"]);
        checkClick(UNNotificationDismissActionIdentifier, @{APHostKey:@"a"}, @[], @[@"complete", @"route:dismiss"]);
        checkClick(UNNotificationDefaultActionIdentifier, @{}, @[], @[@"complete"]);
        checkExpiration();
        puts("PASS: source/time isolation, independent five-second expiration, send failures, click targets, and five delegate ordering cases.");
    }
}
