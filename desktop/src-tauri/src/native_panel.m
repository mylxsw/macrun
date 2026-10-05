#import <AppKit/AppKit.h>

// All UI state stays on AppKit's main thread. Rust owns the worker and controls.
typedef void (*ActionCallback)(const char *);
static void dismiss_panel(bool restore);
@interface MacrunPanel : NSPanel
@end
@implementation MacrunPanel
- (BOOL)canBecomeKeyWindow { return YES; }
- (void)cancelOperation:(id)sender { dismiss_panel(true); }
@end
@interface MacrunPanelController : NSObject <NSWindowDelegate>
@property NSPanel *panel;
@property NSStackView *stack;
@property NSDictionary *model;
@property NSMutableDictionary<NSString *, NSControl *> *controls;
@property ActionCallback callback;
@property NSTimeInterval dismissed;
@property NSString *structure;
@property (weak) NSWindow *previousKeyWindow;
@property BOOL wasActive;
@end
static MacrunPanelController *controller;
static void dismiss_panel(bool restore) {
    [controller.panel orderOut:nil];
    if (restore && controller.previousKeyWindow.visible) {
        // Restore AppKit's key-window chain, including the keyboard focus that a
        // nonactivating panel acquired. Do not activate over another application.
        [controller.previousKeyWindow makeKeyWindow];
        if (controller.wasActive) [NSApp activateIgnoringOtherApps:YES];
    }
}
@implementation MacrunPanelController
- (void)windowDidResignKey:(NSNotification *)note {
    self.dismissed = NSProcessInfo.processInfo.systemUptime;
    [self.panel orderOut:nil];
}
- (void)send:(NSControl *)sender {
    NSString *wire = sender.identifier;
    if (!wire || !self.callback) return;
    // Request feedback is immediate, confirmed state comes back from Rust.
    if (![wire containsString:@"open"] && ![wire containsString:@"quit"]) sender.enabled = NO;
    self.callback(wire.UTF8String);
}
- (NSTextField *)text:(NSString *)value size:(CGFloat)size weight:(NSFontWeight)weight {
    NSTextField *label = [NSTextField labelWithString:value ?: @""];
    label.font = [NSFont systemFontOfSize:size weight:weight];
    label.lineBreakMode = NSLineBreakByTruncatingTail;
    label.maximumNumberOfLines = 1;
    [label setContentCompressionResistancePriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];
    return label;
}
- (NSButton *)button:(NSString *)title action:(NSDictionary *)action {
    NSButton *button = [NSButton buttonWithTitle:title target:self action:@selector(send:)];
    button.bezelStyle = NSBezelStyleRounded;
    NSData *data = [NSJSONSerialization dataWithJSONObject:action options:0 error:nil];
    button.identifier = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    return button;
}
- (void)add:(NSView *)view {
    [self.stack addArrangedSubview:view];
    [view.widthAnchor constraintEqualToAnchor:self.stack.widthAnchor].active = YES;
}
- (void)separator {
    NSBox *line = [NSBox new]; line.boxType = NSBoxSeparator;
    [self add:line];
}
- (NSStackView *)row:(NSArray<NSView *> *)views {
    NSStackView *row = [NSStackView stackViewWithViews:views];
    row.orientation = NSUserInterfaceLayoutOrientationHorizontal;
    row.alignment = NSLayoutAttributeCenterY;
    row.spacing = 10;
    row.distribution = NSStackViewDistributionFill;
    if ([views.firstObject isKindOfClass:NSTextField.class])
        [views.firstObject setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];
    return row;
}
- (void)rebuild {
    for (NSView *view in self.stack.arrangedSubviews.copy) {
        [self.stack removeArrangedSubview:view]; [view removeFromSuperview];
    }
    [self.controls removeAllObjects];
    NSTextField *heading = [self text:self.model[@"title"] size:15 weight:NSFontWeightSemibold];
    heading.identifier = @"heading";
    self.controls[@"heading"] = heading;
    NSButton *stop = [self button:@"全部停止" action:@{@"action":@"stop_all"}];
    self.controls[@"stop_all"] = stop;
    [self add:[self row:@[heading, stop]]];
    NSTextField *subtitle = [self text:self.model[@"subtitle"] size:12 weight:NSFontWeightRegular];
    subtitle.textColor = NSColor.secondaryLabelColor;
    self.controls[@"subtitle"] = subtitle;
    [self add:subtitle];
    [self separator];
    for (NSDictionary *task in self.model[@"tasks"]) {
        NSButton *taskButton = [self button:task[@"title"] action:@{@"action":@"open", @"route":[@"tasks:" stringByAppendingString:task[@"id"]]}];
        taskButton.lineBreakMode = NSLineBreakByTruncatingMiddle;
        taskButton.bordered = NO;
        taskButton.alignment = NSTextAlignmentLeft;
        taskButton.font = [NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];
        taskButton.toolTip = task[@"title"];
        [self add:taskButton];
        if ([task[@"approval"] boolValue]) {
            NSDictionary *allow = @{@"action":@"approve",@"args":@{@"task_id":task[@"id"],@"allow":@YES,@"scope":@"once"}};
            NSDictionary *deny = @{@"action":@"approve",@"args":@{@"task_id":task[@"id"],@"allow":@NO,@"scope":@"once"}};
            NSButton *yes = [self button:@"允许这一次" action:allow];
            NSButton *no = [self button:@"拒绝" action:deny];
            self.controls[[ @"approve:" stringByAppendingString:task[@"id"] ]] = yes;
            self.controls[[ @"deny:" stringByAppendingString:task[@"id"] ]] = no;
            NSButton *similar = [self button:@"15 分钟内同类" action:@{@"action":@"approve",@"args":@{@"task_id":task[@"id"],@"allow":@YES,@"scope":@"similar"}}];
            self.controls[[ @"similar:" stringByAppendingString:task[@"id"] ]] = similar;
            [self add:[self row:@[yes, similar, no]]];
        }
    }
    if ([self.model[@"tasks"] count]) [self separator];
    for (NSArray *item in @[@[@"pause", @"接收新任务"], @[@"desktop", @"允许 Agent 操作桌面"]]) {
        NSTextField *label = [self text:item[1] size:13 weight:NSFontWeightMedium];
        NSSwitch *toggle = [NSSwitch new];
        toggle.target = self; toggle.action = @selector(send:);
        toggle.accessibilityLabel = item[1];
        self.controls[item[0]] = toggle;
        [self add:[self row:@[label, toggle]]];
    }
    [self separator];
    NSButton *open = [self button:@"打开 Macrun" action:@{@"action":@"open",@"route":@"live"}];
    NSButton *settings = [self button:@"设置…" action:@{@"action":@"open",@"route":@"settings"}];
    NSButton *quit = [self button:@"退出" action:@{@"action":@"quit"}];
    [self add:[self row:@[open, settings, quit]]];
    NSTextField *error = [self text:@"" size:12 weight:NSFontWeightRegular];
    error.maximumNumberOfLines = 3; error.lineBreakMode = NSLineBreakByWordWrapping;
    error.textColor = NSColor.systemRedColor;
    self.controls[@"error"] = error;
    [self add:error];
}
- (void)update:(NSDictionary *)model {
    self.model = model;
    // Rebuild only when rows change, never for latency ticks or output updates.
    NSString *structure = [model[@"tasks"] description];
    if (![structure isEqualToString:self.structure]) {
        self.structure = structure; [self rebuild];
    }
    ((NSTextField *)self.controls[@"heading"]).stringValue = model[@"title"] ?: @"Macrun";
    ((NSTextField *)self.controls[@"subtitle"]).stringValue = model[@"subtitle"] ?: @"";
    NSSet *pending = [NSSet setWithArray:model[@"pending"] ?: @[]];
    BOOL available = [model[@"available"] boolValue];
    for (NSString *key in self.controls) {
        NSControl *control = self.controls[key];
        if ([key hasPrefix:@"approve:"] || [key hasPrefix:@"deny:"] || [key hasPrefix:@"similar:"]) {
            NSString *task = [[key componentsSeparatedByString:@":"] lastObject];
            control.enabled = available && ![pending containsObject:[@"approve:" stringByAppendingString:task]];
        }
    }
    self.controls[@"stop_all"].hidden = ![model[@"can_stop"] boolValue];
    self.controls[@"stop_all"].enabled = available && ![pending containsObject:@"stop_all"];
    for (NSString *key in @[@"pause", @"desktop"]) {
        NSSwitch *toggle = (NSSwitch *)self.controls[key];
        toggle.enabled = available && ![pending containsObject:key];
        // Maintain confirmed state while a request is pending.
        toggle.state = [model[key] boolValue] ? NSControlStateValueOn : NSControlStateValueOff;
        NSDictionary *args = [key isEqualToString:@"pause"] ? @{@"paused":model[@"pause"] ?: @NO} : @{@"enabled":[model[@"desktop"] boolValue] ? @NO : @YES};
        NSData *data = [NSJSONSerialization dataWithJSONObject:@{@"action":key,@"args":args} options:0 error:nil];
        toggle.identifier = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    }
    NSString *error = model[@"error"] ?: @"";
    NSTextField *feedback = (NSTextField *)self.controls[@"error"];
    feedback.textColor = error.length ? NSColor.systemRedColor : NSColor.secondaryLabelColor;
    feedback.toolTip = error;
    feedback.stringValue = error.length ? error : (pending.count ? @"正在请求执行器…" : @"");
    feedback.hidden = !feedback.stringValue.length;
    [self.stack layoutSubtreeIfNeeded];
    CGFloat height = MAX(190, self.stack.fittingSize.height + 32);
    NSRect frame = self.panel.frame;
    CGFloat top = NSMaxY(frame);
    frame.size.width = 352;
    frame.size.height = MIN(height, 720);
    frame.origin.y = top - height;
    if (!NSEqualRects(frame, self.panel.frame)) [self.panel setFrame:frame display:YES];
}
@end

void macrun_panel_init(ActionCallback callback) {
    controller = [MacrunPanelController new];
    controller.callback = callback;
    controller.controls = [NSMutableDictionary new];
    controller.panel = [[MacrunPanel alloc] initWithContentRect:NSMakeRect(0, 0, 352, 240)
        styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
        backing:NSBackingStoreBuffered defer:NO];
    controller.panel.title = @"Macrun · 快捷面板";
    controller.panel.floatingPanel = YES;
    controller.panel.level = NSPopUpMenuWindowLevel;
    controller.panel.hidesOnDeactivate = NO;
    controller.panel.opaque = NO;
    controller.panel.backgroundColor = NSColor.clearColor;
    controller.panel.hasShadow = YES;
    controller.panel.contentMinSize = NSMakeSize(352, 190);
    controller.panel.contentMaxSize = NSMakeSize(352, 720);
    controller.panel.delegate = controller;
    controller.panel.collectionBehavior = NSWindowCollectionBehaviorMoveToActiveSpace | NSWindowCollectionBehaviorFullScreenAuxiliary;
    NSVisualEffectView *surface = [NSVisualEffectView new];
    surface.material = NSVisualEffectMaterialPopover;
    surface.blendingMode = NSVisualEffectBlendingModeBehindWindow;
    surface.state = NSVisualEffectStateActive;
    surface.wantsLayer = YES;
    surface.layer.cornerRadius = 12;
    surface.layer.masksToBounds = YES;
    controller.panel.contentView = surface;
    controller.stack = [NSStackView new];
    controller.stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    controller.stack.alignment = NSLayoutAttributeLeading;
    controller.stack.spacing = 10;
    controller.stack.translatesAutoresizingMaskIntoConstraints = NO;
    [surface addSubview:controller.stack];
    [NSLayoutConstraint activateConstraints:@[
        [controller.stack.leadingAnchor constraintEqualToAnchor:surface.leadingAnchor constant:16],
        [controller.stack.widthAnchor constraintEqualToConstant:320],
        [controller.stack.topAnchor constraintEqualToAnchor:surface.topAnchor constant:16]
    ]];
}
void macrun_panel_update(const char *json) {
    if (!controller || !json) return;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    NSDictionary *model = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if ([model isKindOfClass:NSDictionary.class]) [controller update:model];
}
void macrun_panel_show(double x, double y, double width, double height, bool toggle) {
    if (!controller) return;
    if (toggle && (controller.panel.visible || NSProcessInfo.processInfo.systemUptime - controller.dismissed < 0.15)) {
        dismiss_panel(true); return;
    }
    // tray-icon reports physical coordinates using the status item's backing scale.
    CGFloat primaryTop = NSMaxY(NSScreen.screens.firstObject.frame);
    NSScreen *screen = NSScreen.mainScreen;
    NSPoint anchor = NSMakePoint(x / screen.backingScaleFactor + width / screen.backingScaleFactor / 2,
        primaryTop - (y + height) / screen.backingScaleFactor);
    for (NSScreen *candidate in NSScreen.screens) {
        CGFloat scale = candidate.backingScaleFactor;
        NSPoint point = NSMakePoint((x + width / 2) / scale, primaryTop - (y + height) / scale);
        if (NSPointInRect(point, candidate.frame)) { screen = candidate; anchor = point; break; }
    }
    NSRect frame = controller.panel.frame;
    frame.origin.x = MIN(MAX(anchor.x - frame.size.width / 2, NSMinX(screen.visibleFrame) + 8), NSMaxX(screen.visibleFrame) - frame.size.width - 8);
    frame.origin.y = MAX(NSMinY(screen.visibleFrame) + 8, anchor.y - frame.size.height - 6);
    if (!controller.panel.visible) {
        controller.previousKeyWindow = NSApp.keyWindow;
        controller.wasActive = NSApp.active;
    }
    [controller.panel setFrame:frame display:NO];
    BOOL animate = !controller.panel.visible && !NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion;
    controller.panel.alphaValue = animate ? 0 : 1;
    [controller.panel makeKeyAndOrderFront:nil];
    if (animate) [NSAnimationContext runAnimationGroup:^(NSAnimationContext *context) {
        context.duration = 0.12;
        controller.panel.animator.alphaValue = 1;
    } completionHandler:nil];
}
void macrun_panel_hide(void) { [controller.panel orderOut:nil]; }
