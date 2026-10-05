#import <ApplicationServices/ApplicationServices.h>
#import <AppKit/AppKit.h>
#import <IOKit/pwr_mgt/IOPMLib.h>
#include <pthread.h>
#include <stdatomic.h>

// Finder's bundle icon and the running application's Dock tile have separate
// lifetimes. Set the latter explicitly after launch, including after an update.
bool macrun_set_application_icon(const unsigned char *bytes, size_t length) {
    if (![NSThread isMainThread] || !NSApp || !bytes || !length) return false;
    @autoreleasepool {
        NSData *data = [NSData dataWithBytes:bytes length:length];
        NSImage *image = [[NSImage alloc] initWithData:data];
        if (!image || ![image isValid]) {
            [image release];
            return false;
        }
        [NSApp setApplicationIconImage:image];
        [image release];
        return true;
    }
}

static _Atomic unsigned long input_sequence=0;
static _Atomic bool input_available=false;
static CFMachPortRef input_tap=NULL;
static CGEventRef on_input(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *context) {
    (void)proxy; (void)context;
    if(type==kCGEventTapDisabledByTimeout || type==kCGEventTapDisabledByUserInput) {
        if(input_tap) CGEventTapEnable(input_tap,true);
        return event;
    }
    // Ignore synthesized input from a process, including the desktop backend.
    if(CGEventGetIntegerValueField(event,kCGEventSourceUnixProcessID)==0) atomic_fetch_add(&input_sequence,1);
    return event;
}
static void *monitor(void *unused) {
    (void)unused;
    CGEventMask mask=CGEventMaskBit(kCGEventMouseMoved)|CGEventMaskBit(kCGEventLeftMouseDown)|CGEventMaskBit(kCGEventRightMouseDown)|CGEventMaskBit(kCGEventKeyDown)|CGEventMaskBit(kCGEventScrollWheel)|CGEventMaskBit(kCGEventLeftMouseDragged);
    input_tap=CGEventTapCreate(kCGSessionEventTap,kCGHeadInsertEventTap,kCGEventTapOptionListenOnly,mask,on_input,NULL);
    if(!input_tap) return NULL;
    CFRunLoopSourceRef source=CFMachPortCreateRunLoopSource(kCFAllocatorDefault,input_tap,0);
    CFRunLoopAddSource(CFRunLoopGetCurrent(),source,kCFRunLoopCommonModes);
    atomic_store(&input_available,true);CGEventTapEnable(input_tap,true);CFRunLoopRun();return NULL;
}
void macrun_monitor_start(void){pthread_t thread;if(pthread_create(&thread,NULL,monitor,NULL)==0)pthread_detach(thread);}
unsigned long macrun_input_sequence(void){return atomic_load(&input_sequence);}
bool macrun_input_available(void){return atomic_load(&input_available);}
static IOPMAssertionID sleep_assertion=0;
static pthread_mutex_t sleep_assertion_lock=PTHREAD_MUTEX_INITIALIZER;
bool macrun_keep_awake(bool enabled){
    pthread_mutex_lock(&sleep_assertion_lock);
    bool success=true;
    if(enabled && !sleep_assertion) success=IOPMAssertionCreateWithName(kIOPMAssertionTypeNoIdleSleep,kIOPMAssertionLevelOn,CFSTR("Macrun active desktop operation"),&sleep_assertion)==kIOReturnSuccess;
    if(!enabled && sleep_assertion){success=IOPMAssertionRelease(sleep_assertion)==kIOReturnSuccess;if(success)sleep_assertion=0;}
    pthread_mutex_unlock(&sleep_assertion_lock);
    return success;
}
bool macrun_graphical_session(void){
    CFDictionaryRef session=CGSessionCopyCurrentDictionary();if(!session)return false;
    bool active=CFDictionaryGetValue(session,kCGSessionOnConsoleKey)==kCFBooleanTrue && CFDictionaryGetValue(session,kCGSessionLoginDoneKey)==kCFBooleanTrue;
    CFRelease(session);return active;
}
bool macrun_awake_active(void){pthread_mutex_lock(&sleep_assertion_lock);bool active=sleep_assertion!=0;pthread_mutex_unlock(&sleep_assertion_lock);return active;}
