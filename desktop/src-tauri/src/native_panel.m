#import <AppKit/AppKit.h>

// All UI state stays on AppKit's main thread. Rust owns the worker, controls
// and the presentation model (`panel_model.rs`); this file only lays it out.
typedef void (*ActionCallback)(const char *);
static void dismiss_panel(bool restore);
static const CGFloat PanelWidth = 340, Inset = 14;
@class MacrunPanelController;
static MacrunPanelController *controller;

@interface MacrunPanelController : NSObject <NSWindowDelegate>
@property NSPanel *panel;
@property NSStackView *stack;
@property NSDictionary *model;
@property NSMutableDictionary<NSString *, NSView *> *controls;
@property ActionCallback callback;
@property NSTimeInterval dismissed;
@property NSString *structure;
@property (weak) NSWindow *previousKeyWindow;
@property BOOL wasActive;
- (void)sendWire:(NSString *)wire;
@end

static NSString *wireFor(NSDictionary *action) {
    if (![action isKindOfClass:NSDictionary.class]) return nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:action options:0 error:nil];
    return data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
}

@interface MacrunPanel : NSPanel
@end
@implementation MacrunPanel
- (BOOL)canBecomeKeyWindow { return YES; }
- (void)cancelOperation:(id)sender { dismiss_panel(true); }
// The menu rows show these shortcuts; a nonactivating panel has no menu bar.
- (BOOL)performKeyEquivalent:(NSEvent *)event {
    NSEventModifierFlags flags = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    if (flags == NSEventModifierFlagCommand) {
        NSString *key = event.charactersIgnoringModifiers;
        NSDictionary *action = [key isEqualToString:@"o"] ? @{@"action":@"open",@"route":@"live"}
            : [key isEqualToString:@","] ? @{@"action":@"open",@"route":@"settings"}
            : [key isEqualToString:@"q"] ? @{@"action":@"quit"} : nil;
        if (action) { [controller sendWire:wireFor(action)]; return YES; }
    }
    return [super performKeyEquivalent:event];
}
@end

// A clickable full-width row with a hover highlight, like a menu item.
@interface MacrunRow : NSView
@property NSString *wire;
@property NSString *label;
@property BOOL hover;
@end
@implementation MacrunRow
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    for (NSTrackingArea *area in self.trackingAreas.copy) [self removeTrackingArea:area];
    [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect
        options:NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways | NSTrackingInVisibleRect
        owner:self userInfo:nil]];
}
- (void)mouseEntered:(NSEvent *)event { self.hover = YES; self.needsDisplay = YES; }
- (void)mouseExited:(NSEvent *)event { self.hover = NO; self.needsDisplay = YES; }
- (BOOL)acceptsFirstMouse:(NSEvent *)event { return YES; }
- (void)drawRect:(NSRect)rect {
    if (!self.hover || !self.wire) return;
    [[NSColor.labelColor colorWithAlphaComponent:0.08] setFill];
    [[NSBezierPath bezierPathWithRoundedRect:self.bounds xRadius:6 yRadius:6] fill];
}
- (void)mouseDown:(NSEvent *)event {}
- (void)mouseUp:(NSEvent *)event {
    NSPoint point = [self convertPoint:event.locationInWindow fromView:nil];
    if (self.wire && NSPointInRect(point, self.bounds)) [controller sendWire:self.wire];
}
- (BOOL)isAccessibilityElement { return self.wire != nil; }
- (NSAccessibilityRole)accessibilityRole { return NSAccessibilityButtonRole; }
- (NSString *)accessibilityLabel { return self.label; }
- (BOOL)accessibilityPerformPress { if (self.wire) [controller sendWire:self.wire]; return self.wire != nil; }
@end

static void dismiss_panel(bool restore) {
    [controller.panel orderOut:nil];
    if (restore && controller.previousKeyWindow.visible) {
        // Restore AppKit's key-window chain, including the keyboard focus that a
        // nonactivating panel acquired. Do not activate over another application.
        [controller.previousKeyWindow makeKeyWindow];
        if (controller.wasActive) [NSApp activateIgnoringOtherApps:YES];
    }
}

static NSColor *toneColor(NSString *tone) {
    if ([tone isEqualToString:@"run"]) return NSColor.systemBlueColor;
    if ([tone isEqualToString:@"approval"]) return NSColor.systemOrangeColor;
    if ([tone isEqualToString:@"desktop"]) return NSColor.systemOrangeColor;
    if ([tone isEqualToString:@"ok"]) return NSColor.systemGreenColor;
    if ([tone isEqualToString:@"error"]) return NSColor.systemRedColor;
    return NSColor.systemGrayColor;
}

@implementation MacrunPanelController
- (void)windowDidResignKey:(NSNotification *)note {
    self.dismissed = NSProcessInfo.processInfo.systemUptime;
    [self.panel orderOut:nil];
}
- (void)sendWire:(NSString *)wire {
    if (!wire || !self.callback) return;
    self.callback(wire.UTF8String);
}
- (void)send:(NSControl *)sender {
    NSString *wire = sender.identifier;
    if (!wire) return;
    // Request feedback is immediate, confirmed state comes back from Rust.
    if (![wire containsString:@"\"open\""] && ![wire containsString:@"\"quit\""]) sender.enabled = NO;
    [self sendWire:wire];
}
- (void)sendMenu:(NSMenuItem *)item { [self sendWire:item.representedObject]; }

// MARK: building blocks
- (NSTextField *)text:(NSString *)value size:(CGFloat)size weight:(NSFontWeight)weight {
    NSTextField *label = [NSTextField labelWithString:value ?: @""];
    label.font = [NSFont systemFontOfSize:size weight:weight];
    label.lineBreakMode = NSLineBreakByTruncatingTail;
    label.maximumNumberOfLines = 1;
    [label setContentCompressionResistancePriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];
    return label;
}
- (NSTextField *)mono:(NSString *)value size:(CGFloat)size color:(NSColor *)color {
    NSTextField *label = [self text:value size:size weight:NSFontWeightRegular];
    label.font = [NSFont monospacedSystemFontOfSize:size weight:NSFontWeightRegular];
    label.textColor = color;
    return label;
}
- (NSTextField *)keep:(NSTextField *)label as:(NSString *)key {
    self.controls[key] = label;
    return label;
}
- (NSButton *)button:(NSString *)title action:(NSDictionary *)action {
    NSButton *button = [NSButton buttonWithTitle:title target:self action:@selector(send:)];
    button.bezelStyle = NSBezelStyleRounded;
    button.controlSize = NSControlSizeSmall;
    button.font = [NSFont systemFontOfSize:12];
    button.identifier = wireFor(action);
    [button setContentHuggingPriority:NSLayoutPriorityRequired forOrientation:NSLayoutConstraintOrientationHorizontal];
    [button setContentCompressionResistancePriority:NSLayoutPriorityRequired forOrientation:NSLayoutConstraintOrientationHorizontal];
    return button;
}
- (NSImageView *)symbol:(NSString *)name color:(NSColor *)color {
    NSImage *image = [NSImage imageWithSystemSymbolName:name accessibilityDescription:nil];
    NSImageView *view = [NSImageView imageViewWithImage:image ?: [NSImage new]];
    view.symbolConfiguration = [NSImageSymbolConfiguration configurationWithPointSize:12 weight:NSFontWeightMedium];
    view.contentTintColor = color;
    [view.widthAnchor constraintEqualToConstant:16].active = YES;
    return view;
}
- (NSView *)dot:(NSColor *)color {
    NSView *dot = [NSView new];
    dot.wantsLayer = YES;
    dot.layer.backgroundColor = color.CGColor;
    dot.layer.cornerRadius = 4;
    [dot.widthAnchor constraintEqualToConstant:8].active = YES;
    [dot.heightAnchor constraintEqualToConstant:8].active = YES;
    return dot;
}
- (NSStackView *)row:(NSArray<NSView *> *)views spacing:(CGFloat)spacing {
    NSStackView *row = [NSStackView stackViewWithViews:views];
    row.orientation = NSUserInterfaceLayoutOrientationHorizontal;
    row.alignment = NSLayoutAttributeCenterY;
    row.spacing = spacing;
    row.distribution = NSStackViewDistributionFill;
    return row;
}
- (NSStackView *)column:(NSArray<NSView *> *)views spacing:(CGFloat)spacing {
    NSStackView *column = [NSStackView stackViewWithViews:views];
    column.orientation = NSUserInterfaceLayoutOrientationVertical;
    column.alignment = NSLayoutAttributeLeading;
    column.spacing = spacing;
    for (NSView *view in views)
        [view.widthAnchor constraintEqualToAnchor:column.widthAnchor].active = YES;
    [column setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];
    [column setContentCompressionResistancePriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];
    return column;
}
- (NSView *)spacer {
    NSView *spacer = [NSView new];
    [spacer setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];
    return spacer;
}
- (void)add:(NSView *)view inset:(CGFloat)inset {
    [self.stack addArrangedSubview:view];
    [view.widthAnchor constraintEqualToAnchor:self.stack.widthAnchor constant:-2 * inset].active = YES;
}
- (void)separator {
    NSBox *line = [NSBox new]; line.boxType = NSBoxSeparator;
    [self add:line inset:Inset];
}
- (MacrunRow *)clickable:(NSView *)content action:(NSDictionary *)action label:(NSString *)label {
    MacrunRow *row = [MacrunRow new];
    row.wire = wireFor(action);
    row.label = label;
    content.translatesAutoresizingMaskIntoConstraints = NO;
    [row addSubview:content];
    [NSLayoutConstraint activateConstraints:@[
        [content.leadingAnchor constraintEqualToAnchor:row.leadingAnchor constant:8],
        [content.trailingAnchor constraintEqualToAnchor:row.trailingAnchor constant:-8],
        [content.topAnchor constraintEqualToAnchor:row.topAnchor constant:5],
        [content.bottomAnchor constraintEqualToAnchor:row.bottomAnchor constant:-5],
    ]];
    return row;
}
- (MacrunRow *)menuRow:(NSString *)title shortcut:(NSString *)shortcut action:(NSDictionary *)action {
    NSTextField *name = [self text:title size:13 weight:NSFontWeightRegular];
    NSTextField *key = [self text:shortcut size:12 weight:NSFontWeightRegular];
    key.textColor = NSColor.tertiaryLabelColor;
    return [self clickable:[self row:@[name, [self spacer], key] spacing:8] action:action label:title];
}

// MARK: sections
- (void)buildHeader {
    NSDictionary *m = self.model;
    NSView *dot = [self dot:toneColor(m[@"tone"])];
    self.controls[@"dot"] = dot;
    NSTextField *title = [self keep:[self text:m[@"title"] size:14 weight:NSFontWeightSemibold] as:@"title"];
    NSTextField *subtitle = [self keep:[self text:m[@"subtitle"] size:12 weight:NSFontWeightRegular] as:@"subtitle"];
    subtitle.textColor = NSColor.secondaryLabelColor;
    NSMutableArray *views = [@[dot, [self column:@[title, subtitle] spacing:1]] mutableCopy];
    NSDictionary *primary = m[@"primary"];
    if ([primary isKindOfClass:NSDictionary.class]) {
        NSButton *button = [self button:primary[@"label"] action:primary[@"action"]];
        if ([primary[@"danger"] boolValue]) {
            button.image = [NSImage imageWithSystemSymbolName:@"stop.fill" accessibilityDescription:nil];
            button.imagePosition = NSImageLeading;
            button.contentTintColor = NSColor.systemRedColor;
            button.attributedTitle = [[NSAttributedString alloc] initWithString:primary[@"label"]
                attributes:@{NSForegroundColorAttributeName: NSColor.systemRedColor,
                             NSFontAttributeName: [NSFont systemFontOfSize:12 weight:NSFontWeightSemibold]}];
            button.toolTip = @"取消所有任务、暂停接收并关闭桌面控制（⌃⌥⌘.）";
        }
        if ([primary[@"primary"] boolValue]) button.keyEquivalent = @"\r";
        self.controls[@"primary"] = button;
        [views addObject:button];
    }
    NSDictionary *approval = m[@"approval"];
    if ([approval isKindOfClass:NSDictionary.class] && [approval[@"count"] integerValue] > 1) {
        NSTextField *count = [self text:[NSString stringWithFormat:@"1 / %@", approval[@"count"]] size:12 weight:NSFontWeightRegular];
        count.textColor = NSColor.tertiaryLabelColor;
        [views addObject:count];
    }
    NSStackView *header = [self row:views spacing:10];
    header.alignment = NSLayoutAttributeCenterY;
    [self add:header inset:Inset];
}
- (void)buildApproval:(NSDictionary *)a {
    NSString *task = a[@"id"];
    NSTextField *command = [NSTextField wrappingLabelWithString:a[@"command"] ?: @""];
    command.font = [NSFont monospacedSystemFontOfSize:11.5 weight:NSFontWeightRegular];
    command.maximumNumberOfLines = 3;
    command.lineBreakMode = NSLineBreakByCharWrapping;
    command.cell.truncatesLastVisibleLine = YES;
    command.preferredMaxLayoutWidth = PanelWidth - 2 * Inset - 12;
    command.selectable = YES;
    command.toolTip = a[@"command"];
    [command setContentCompressionResistancePriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];
    NSMutableArray *lines = [@[command] mutableCopy];
    NSArray *reasons = a[@"reasons"];
    if (reasons.count) {
        NSTextField *why = [self text:[reasons componentsJoinedByString:@" · "] size:11.5 weight:NSFontWeightMedium];
        why.textColor = NSColor.systemOrangeColor;
        [lines addObject:why];
    }
    if ([a[@"cwd"] length]) [lines addObject:[self mono:a[@"cwd"] size:11 color:NSColor.secondaryLabelColor]];
    NSDictionary *deny = @{@"action":@"approve",@"args":@{@"task_id":task,@"allow":@NO,@"scope":@"once"}};
    NSDictionary *once = @{@"action":@"approve",@"args":@{@"task_id":task,@"allow":@YES,@"scope":@"once"}};
    NSButton *no = [self button:@"拒绝" action:deny];
    NSButton *yes = [self button:@"允许一次" action:once];
    yes.keyEquivalent = @"\r";
    NSPopUpButton *more = [[NSPopUpButton alloc] initWithFrame:NSZeroRect pullsDown:YES];
    more.controlSize = NSControlSizeSmall;
    more.font = [NSFont systemFontOfSize:12];
    more.toolTip = @"更多允许方式";
    [more addItemWithTitle:@"更多"];
    for (NSArray *scope in @[@[@"similar", a[@"similar"] ?: @""], @[@"session", a[@"session"] ?: @""]]) {
        NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:scope[1] action:@selector(sendMenu:) keyEquivalent:@""];
        item.target = self;
        item.representedObject = wireFor(@{@"action":@"approve",@"args":@{@"task_id":task,@"allow":@YES,@"scope":scope[0]}});
        [more.menu addItem:item];
    }
    self.controls[[@"deny:" stringByAppendingString:task]] = no;
    self.controls[[@"approve:" stringByAppendingString:task]] = yes;
    self.controls[[@"scope:" stringByAppendingString:task]] = more;
    [lines addObject:[self row:@[no, [self spacer], more, yes] spacing:6]];
    NSStackView *body = [self column:lines spacing:6];
    NSBox *box = [NSBox new];
    box.boxType = NSBoxCustom;
    box.cornerRadius = 9;
    box.borderWidth = 0.5;
    box.borderColor = [NSColor.systemOrangeColor colorWithAlphaComponent:0.45];
    box.fillColor = [NSColor.systemOrangeColor colorWithAlphaComponent:0.10];
    box.contentViewMargins = NSMakeSize(10, 9);
    body.translatesAutoresizingMaskIntoConstraints = NO;
    [box.contentView addSubview:body];
    [NSLayoutConstraint activateConstraints:@[
        [body.leadingAnchor constraintEqualToAnchor:box.contentView.leadingAnchor],
        [body.trailingAnchor constraintEqualToAnchor:box.contentView.trailingAnchor],
        [body.topAnchor constraintEqualToAnchor:box.contentView.topAnchor],
        [body.bottomAnchor constraintEqualToAnchor:box.contentView.bottomAnchor],
    ]];
    [self add:box inset:Inset - 4];
}
- (void)buildRows {
    NSArray *rows = self.model[@"rows"];
    for (NSUInteger i = 0; i < rows.count; i++) {
        NSDictionary *r = rows[i];
        NSString *prefix = [NSString stringWithFormat:@"row:%lu:", (unsigned long)i];
        BOOL desktop = [r[@"desktop"] boolValue];
        NSView *lead;
        if (desktop) {
            lead = [self symbol:@"cursorarrow.rays" color:NSColor.systemOrangeColor];
        } else {
            NSProgressIndicator *spin = [NSProgressIndicator new];
            spin.style = NSProgressIndicatorStyleSpinning;
            spin.controlSize = NSControlSizeSmall;
            spin.displayedWhenStopped = NO;
            [spin.widthAnchor constraintEqualToConstant:14].active = YES;
            [spin.heightAnchor constraintEqualToConstant:14].active = YES;
            [spin startAnimation:nil];
            lead = spin;
        }
        NSTextField *name = [self text:desktop ? @"桌面操作" : r[@"name"] size:13 weight:NSFontWeightSemibold];
        if (desktop) name.textColor = NSColor.systemOrangeColor;
        NSTextField *tag = [self keep:[self text:r[@"tag"] size:11.5 weight:NSFontWeightRegular] as:[prefix stringByAppendingString:@"tag"]];
        tag.textColor = NSColor.secondaryLabelColor;
        NSTextField *elapsed = [self keep:[self mono:r[@"elapsed"] size:11.5 color:NSColor.secondaryLabelColor] as:[prefix stringByAppendingString:@"elapsed"]];
        [elapsed setContentCompressionResistancePriority:NSLayoutPriorityRequired forOrientation:NSLayoutConstraintOrientationHorizontal];
        NSStackView *top = [self row:@[name, tag, [self spacer], elapsed] spacing:6];
        NSTextField *step = [self keep:[self mono:r[@"step"] size:11.5 color:NSColor.secondaryLabelColor] as:[prefix stringByAppendingString:@"step"]];
        NSTextField *tail = [self keep:[self mono:r[@"tail"] size:11 color:NSColor.secondaryLabelColor] as:[prefix stringByAppendingString:@"tail"]];
        tail.hidden = ![r[@"tail"] length];
        NSStackView *lines = [self column:@[top, step, tail] spacing:1];
        NSStackView *content = [self row:@[lead, lines] spacing:9];
        content.alignment = NSLayoutAttributeTop;
        NSString *label = [NSString stringWithFormat:@"%@ %@，查看任务详情", r[@"name"], r[@"step"]];
        [self add:[self clickable:content action:@{@"action":@"open",@"route":[@"tasks:" stringByAppendingString:r[@"id"]]} label:label] inset:Inset - 8];
    }
    NSInteger more = [self.model[@"more"] integerValue];
    if (more > 0) {
        NSTextField *text = [self text:[NSString stringWithFormat:@"还有 %ld 个项目", (long)more] size:12 weight:NSFontWeightRegular];
        text.textColor = NSColor.secondaryLabelColor;
        [self add:[self clickable:text action:@{@"action":@"open",@"route":@"live"} label:text.stringValue] inset:Inset - 8];
    }
}
- (void)buildRecent {
    NSArray *recent = self.model[@"recent"];
    NSTextField *heading = [self text:@"最近" size:11 weight:NSFontWeightSemibold];
    heading.textColor = NSColor.tertiaryLabelColor;
    [self add:heading inset:Inset];
    for (NSDictionary *r in recent) {
        NSTextField *name = [self text:r[@"name"] size:13 weight:NSFontWeightMedium];
        NSString *detail = [r[@"tag"] length] ? [NSString stringWithFormat:@"%@ · %@", r[@"tag"], r[@"detail"]] : r[@"detail"];
        NSTextField *info = [self text:detail size:11.5 weight:NSFontWeightRegular];
        info.textColor = NSColor.secondaryLabelColor;
        NSTextField *when = [self text:r[@"when"] size:11.5 weight:NSFontWeightRegular];
        when.textColor = NSColor.secondaryLabelColor;
        NSStackView *content = [self row:@[[self symbol:@"checkmark" color:NSColor.secondaryLabelColor], name, info, [self spacer], when] spacing:7];
        [self add:[self clickable:content action:@{@"action":@"open",@"route":[@"tasks:" stringByAppendingString:r[@"id"]]} label:r[@"name"]] inset:Inset - 8];
    }
}
- (void)buildProblems {
    for (NSDictionary *p in self.model[@"problems"]) {
        BOOL pause = [p[@"button"] isEqualToString:@"恢复"];
        NSImageView *icon = [self symbol:pause ? @"pause.circle" : @"exclamationmark.triangle" color:pause ? NSColor.secondaryLabelColor : NSColor.systemOrangeColor];
        NSTextField *text = [self text:p[@"text"] size:12.5 weight:NSFontWeightRegular];
        NSTextField *detail = [self text:p[@"detail"] size:11.5 weight:NSFontWeightRegular];
        detail.textColor = NSColor.secondaryLabelColor;
        NSButton *button = [self button:p[@"button"] action:p[@"action"]];
        if (pause) self.controls[@"problem:pause"] = button;
        [self add:[self row:@[icon, [self column:@[text, detail] spacing:1], button] spacing:9] inset:Inset];
    }
}
- (void)rebuild {
    for (NSView *view in self.stack.arrangedSubviews.copy) {
        [self.stack removeArrangedSubview:view]; [view removeFromSuperview];
    }
    [self.controls removeAllObjects];
    NSDictionary *m = self.model;
    [self buildHeader];
    NSDictionary *approval = m[@"approval"];
    BOOL hasApproval = [approval isKindOfClass:NSDictionary.class];
    if (hasApproval) [self buildApproval:approval];
    BOOL rows = [m[@"rows"] count] > 0, recent = [m[@"recent"] count] > 0, problems = [m[@"problems"] count] > 0;
    if (rows || recent || problems) [self separator];
    if (problems) [self buildProblems];
    if (rows) [self buildRows];
    if (recent) [self buildRecent];
    if ([m[@"today"] length] && !hasApproval) {
        [self separator];
        NSTextField *today = [self keep:[self text:m[@"today"] size:12.5 weight:NSFontWeightRegular] as:@"today"];
        today.textColor = NSColor.secondaryLabelColor;
        NSTextField *chevron = [self text:@"›" size:13 weight:NSFontWeightRegular];
        chevron.textColor = NSColor.tertiaryLabelColor;
        [self add:[self clickable:[self row:@[today, [self spacer], chevron] spacing:6] action:@{@"action":@"open",@"route":@"tasks"} label:@"查看今天的活动"] inset:Inset - 8];
    }
    [self separator];
    for (NSArray *item in @[@[@"pause", @"接收新任务"], @[@"desktop", @"允许 Agent 操作桌面"]]) {
        NSTextField *label = [self text:item[1] size:13 weight:NSFontWeightRegular];
        NSSwitch *toggle = [NSSwitch new];
        toggle.controlSize = NSControlSizeSmall;
        toggle.target = self; toggle.action = @selector(send:);
        toggle.accessibilityLabel = item[1];
        self.controls[item[0]] = toggle;
        [self add:[self row:@[label, [self spacer], toggle] spacing:10] inset:Inset];
    }
    [self separator];
    [self add:[self menuRow:@"打开 Macrun" shortcut:@"⌘O" action:@{@"action":@"open",@"route":@"live"}] inset:Inset - 8];
    [self add:[self menuRow:@"设置…" shortcut:@"⌘," action:@{@"action":@"open",@"route":@"settings"}] inset:Inset - 8];
    [self add:[self menuRow:@"退出 Macrun" shortcut:@"⌘Q" action:@{@"action":@"quit"}] inset:Inset - 8];
    NSTextField *error = [self text:@"" size:12 weight:NSFontWeightRegular];
    error.maximumNumberOfLines = 3; error.lineBreakMode = NSLineBreakByWordWrapping;
    self.controls[@"error"] = error;
    [self add:error inset:Inset];
}

// MARK: updates
- (void)setText:(NSString *)key value:(id)value {
    NSTextField *label = (NSTextField *)self.controls[key];
    if ([label isKindOfClass:NSTextField.class] && [value isKindOfClass:NSString.class]) {
        label.stringValue = value;
        label.toolTip = [value length] > 40 ? value : nil;
    }
}
- (void)update:(NSDictionary *)model {
    self.model = model;
    // Rebuild only when the layout changes, never for latency ticks or output.
    NSString *structure = model[@"structure"] ?: @"";
    if (![structure isEqualToString:self.structure]) {
        self.structure = structure; [self rebuild];
    }
    [self setText:@"title" value:model[@"title"]];
    [self setText:@"subtitle" value:model[@"subtitle"]];
    [self setText:@"today" value:model[@"today"]];
    self.controls[@"dot"].layer.backgroundColor = toneColor(model[@"tone"]).CGColor;
    NSArray *rows = model[@"rows"];
    for (NSUInteger i = 0; i < rows.count; i++) {
        NSDictionary *r = rows[i];
        NSString *prefix = [NSString stringWithFormat:@"row:%lu:", (unsigned long)i];
        for (NSString *field in @[@"elapsed", @"step", @"tail", @"tag"])
            [self setText:[prefix stringByAppendingString:field] value:r[field]];
        self.controls[[prefix stringByAppendingString:@"tail"]].hidden = ![r[@"tail"] length];
    }
    NSSet *pending = [NSSet setWithArray:model[@"pending"] ?: @[]];
    BOOL available = [model[@"available"] boolValue];
    for (NSString *key in self.controls) {
        NSView *view = self.controls[key];
        NSString *task = [[key componentsSeparatedByString:@":"] lastObject];
        if ([key hasPrefix:@"approve:"] || [key hasPrefix:@"deny:"] || [key hasPrefix:@"scope:"])
            ((NSControl *)view).enabled = available && ![pending containsObject:[@"approve:" stringByAppendingString:task]];
    }
    NSDictionary *primary = model[@"primary"];
    NSButton *primaryButton = (NSButton *)self.controls[@"primary"];
    if (primaryButton && [primary isKindOfClass:NSDictionary.class]) {
        NSString *action = primary[@"action"][@"action"];
        primaryButton.enabled = ![action isEqualToString:@"open"] ? available && ![pending containsObject:action] : YES;
    }
    ((NSControl *)self.controls[@"problem:pause"]).enabled = available && ![pending containsObject:@"pause"];
    for (NSString *key in @[@"pause", @"desktop"]) {
        NSSwitch *toggle = (NSSwitch *)self.controls[key];
        toggle.enabled = available && ![pending containsObject:key];
        // Maintain confirmed state while a request is pending.
        toggle.state = [model[key] boolValue] ? NSControlStateValueOn : NSControlStateValueOff;
        NSDictionary *args = [key isEqualToString:@"pause"] ? @{@"paused":model[@"pause"] ?: @NO} : @{@"enabled":[model[@"desktop"] boolValue] ? @NO : @YES};
        toggle.identifier = wireFor(@{@"action":key,@"args":args});
    }
    NSString *error = model[@"error"] ?: @"";
    NSTextField *feedback = (NSTextField *)self.controls[@"error"];
    feedback.textColor = error.length ? NSColor.systemRedColor : NSColor.secondaryLabelColor;
    feedback.toolTip = error;
    feedback.stringValue = error.length ? error : (pending.count ? @"正在请求执行器…" : @"");
    feedback.hidden = !feedback.stringValue.length;
    [self.stack layoutSubtreeIfNeeded];
    CGFloat height = MAX(160, self.stack.fittingSize.height + 24);
    NSRect frame = self.panel.frame;
    CGFloat top = NSMaxY(frame);
    frame.size.width = PanelWidth;
    frame.size.height = MIN(height, 720);
    frame.origin.y = top - frame.size.height;
    if (!NSEqualRects(frame, self.panel.frame)) [self.panel setFrame:frame display:YES];
}
@end

void macrun_panel_init(ActionCallback callback) {
    controller = [MacrunPanelController new];
    controller.callback = callback;
    controller.controls = [NSMutableDictionary new];
    controller.panel = [[MacrunPanel alloc] initWithContentRect:NSMakeRect(0, 0, PanelWidth, 240)
        styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
        backing:NSBackingStoreBuffered defer:NO];
    controller.panel.title = @"Macrun · 快捷面板";
    controller.panel.floatingPanel = YES;
    controller.panel.level = NSPopUpMenuWindowLevel;
    controller.panel.hidesOnDeactivate = NO;
    controller.panel.opaque = NO;
    controller.panel.backgroundColor = NSColor.clearColor;
    controller.panel.hasShadow = YES;
    controller.panel.contentMinSize = NSMakeSize(PanelWidth, 160);
    controller.panel.contentMaxSize = NSMakeSize(PanelWidth, 720);
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
    controller.stack.alignment = NSLayoutAttributeCenterX;
    controller.stack.spacing = 8;
    controller.stack.translatesAutoresizingMaskIntoConstraints = NO;
    [surface addSubview:controller.stack];
    [NSLayoutConstraint activateConstraints:@[
        [controller.stack.leadingAnchor constraintEqualToAnchor:surface.leadingAnchor],
        [controller.stack.widthAnchor constraintEqualToConstant:PanelWidth],
        [controller.stack.topAnchor constraintEqualToAnchor:surface.topAnchor constant:13]
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
