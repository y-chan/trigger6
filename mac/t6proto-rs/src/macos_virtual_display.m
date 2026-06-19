#import <CoreGraphics/CoreGraphics.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#import <dispatch/dispatch.h>
#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

@class CGVirtualDisplayDescriptor;
@class CGVirtualDisplay;

@interface CGVirtualDisplayMode : NSObject
- (instancetype)initWithWidth:(NSUInteger)width height:(NSUInteger)height refreshRate:(CGFloat)refreshRate;
@end

@interface CGVirtualDisplaySettings : NSObject
@property(retain, nonatomic) NSArray *modes;
@property(nonatomic) unsigned int hiDPI;
- (instancetype)init;
@end

@interface CGVirtualDisplayDescriptor : NSObject
@property(retain, nonatomic) dispatch_queue_t queue;
@property(retain, nonatomic) NSString *name;
@property(nonatomic) unsigned int maxPixelsHigh;
@property(nonatomic) unsigned int maxPixelsWide;
@property(nonatomic) CGSize sizeInMillimeters;
@property(nonatomic) unsigned int serialNum;
@property(nonatomic) unsigned int productID;
@property(nonatomic) unsigned int vendorID;
@property(copy, nonatomic) void (^terminationHandler)(id, CGVirtualDisplay *);
- (instancetype)init;
@end

@interface CGVirtualDisplay : NSObject
@property(readonly, nonatomic) CGDirectDisplayID displayID;
- (instancetype)initWithDescriptor:(CGVirtualDisplayDescriptor *)descriptor;
- (BOOL)applySettings:(CGVirtualDisplaySettings *)settings;
@end

typedef void (*t6_vd_frame_callback)(
    uint32_t pixel_format,
    const uint8_t *plane0,
    size_t plane0_byte_count,
    size_t width,
    size_t height,
    size_t plane0_stride,
    const uint8_t *plane1,
    size_t plane1_byte_count,
    size_t plane1_width,
    size_t plane1_height,
    size_t plane1_stride,
    void *user_data);

static CGVirtualDisplay *g_display = nil;
static CGDisplayStreamRef g_stream = NULL;
static dispatch_queue_t g_queue = NULL;
static const char *g_last_error = "not started";

typedef CGDisplayStreamRef (*CGDisplayStreamCreateWithDispatchQueueFn)(
    CGDirectDisplayID display,
    size_t outputWidth,
    size_t outputHeight,
    int32_t pixelFormat,
    CFDictionaryRef properties,
    dispatch_queue_t queue,
    CGDisplayStreamFrameAvailableHandler handler);
typedef CGError (*CGDisplayStreamStartFn)(CGDisplayStreamRef displayStream);
typedef CGError (*CGDisplayStreamStopFn)(CGDisplayStreamRef displayStream);

static uint32_t t6_fourcc_420f(void) {
    return ((uint32_t)'4' << 24) | ((uint32_t)'2' << 16) | ((uint32_t)'0' << 8) | (uint32_t)'f';
}

static uint32_t t6_fourcc_420v(void) {
    return ((uint32_t)'4' << 24) | ((uint32_t)'2' << 16) | ((uint32_t)'0' << 8) | (uint32_t)'v';
}

static CFStringRef t6_cg_string_symbol(const char *name) {
    CFStringRef *symbol = (CFStringRef *)dlsym(RTLD_DEFAULT, name);
    if (symbol == NULL) {
        return NULL;
    }
    return *symbol;
}

const char *t6_vd_last_error(void) {
    return g_last_error;
}

uint32_t t6_vd_start(
    size_t width,
    size_t height,
    double refresh_rate,
    uint32_t pixel_format,
    t6_vd_frame_callback callback,
    void *user_data) {
    @autoreleasepool {
        if (g_display != nil || g_stream != NULL) {
            g_last_error = "virtual display is already running";
            return 0;
        }

        g_queue = dispatch_queue_create("dev.trigger6.virtual-display", DISPATCH_QUEUE_SERIAL);

        CGVirtualDisplayDescriptor *descriptor = [[CGVirtualDisplayDescriptor alloc] init];
        descriptor.queue = g_queue;
        descriptor.name = @"Trigger6 Virtual Display";
        size_t max_dimension = width > height ? width : height;
        descriptor.maxPixelsWide = (unsigned int)max_dimension;
        descriptor.maxPixelsHigh = (unsigned int)max_dimension;
        descriptor.sizeInMillimeters = width >= height ? CGSizeMake(530, 300) : CGSizeMake(300, 530);
        descriptor.vendorID = 0x0711;
        descriptor.productID = 0x5601;
        descriptor.serialNum = 1;
        descriptor.terminationHandler = ^(id _Nonnull _displayID, CGVirtualDisplay *_Nonnull display) {
            (void)_displayID;
            (void)display;
        };

        g_display = [[CGVirtualDisplay alloc] initWithDescriptor:descriptor];
        if (g_display == nil || g_display.displayID == 0) {
            g_last_error = "CGVirtualDisplay init failed; private virtual-display entitlement may be missing";
            g_display = nil;
            g_queue = NULL;
            return 0;
        }

        CGVirtualDisplayMode *mode = [[CGVirtualDisplayMode alloc]
            initWithWidth:(NSUInteger)width
                  height:(NSUInteger)height
             refreshRate:(CGFloat)refresh_rate];
        CGVirtualDisplaySettings *settings = [[CGVirtualDisplaySettings alloc] init];
        settings.modes = @[ mode ];
        settings.hiDPI = 0;

        if (![g_display applySettings:settings]) {
            g_last_error = "CGVirtualDisplay applySettings failed; requested mode may be unsupported by private virtual-display API";
            g_display = nil;
            g_queue = NULL;
            return 0;
        }

        CGDirectDisplayID display_id = g_display.displayID;
        CGDisplayStreamCreateWithDispatchQueueFn create_stream =
            (CGDisplayStreamCreateWithDispatchQueueFn)dlsym(RTLD_DEFAULT, "CGDisplayStreamCreateWithDispatchQueue");
        CGDisplayStreamStartFn start_stream =
            (CGDisplayStreamStartFn)dlsym(RTLD_DEFAULT, "CGDisplayStreamStart");
        if (create_stream == NULL || start_stream == NULL) {
            g_last_error = "CGDisplayStream symbols are unavailable at runtime";
            g_display = nil;
            g_queue = NULL;
            return 0;
        }

        NSMutableDictionary *stream_properties = [NSMutableDictionary dictionary];
        CFStringRef show_cursor_key = t6_cg_string_symbol("kCGDisplayStreamShowCursor");
        if (show_cursor_key != NULL) {
            stream_properties[(__bridge NSString *)show_cursor_key] = @YES;
        }
        CFStringRef queue_depth_key = t6_cg_string_symbol("kCGDisplayStreamQueueDepth");
        if (queue_depth_key != NULL) {
            stream_properties[(__bridge NSString *)queue_depth_key] = @2;
        }
        CFStringRef matrix_key = t6_cg_string_symbol("kCGDisplayStreamYCbCrMatrix");
        CFStringRef matrix_709 = t6_cg_string_symbol("kCGDisplayStreamYCbCrMatrix_ITU_R_709_2");
        if (matrix_key != NULL && matrix_709 != NULL &&
            (pixel_format == t6_fourcc_420f() || pixel_format == t6_fourcc_420v())) {
            stream_properties[(__bridge NSString *)matrix_key] = (__bridge NSString *)matrix_709;
        }

        g_stream = create_stream(
            display_id,
            width,
            height,
            (int32_t)pixel_format,
            (__bridge CFDictionaryRef)stream_properties,
            g_queue,
            ^(CGDisplayStreamFrameStatus status,
              uint64_t display_time,
              IOSurfaceRef frame_surface,
              CGDisplayStreamUpdateRef update_ref) {
                (void)display_time;
                (void)update_ref;

                if (status != kCGDisplayStreamFrameStatusFrameComplete ||
                    frame_surface == NULL ||
                    callback == NULL) {
                    return;
                }

                IOSurfaceLock(frame_surface, kIOSurfaceLockReadOnly, NULL);
                size_t plane_count = IOSurfaceGetPlaneCount(frame_surface);
                if (plane_count >= 2 &&
                    (pixel_format == t6_fourcc_420f() || pixel_format == t6_fourcc_420v())) {
                    uint8_t *y_base = (uint8_t *)IOSurfaceGetBaseAddressOfPlane(frame_surface, 0);
                    uint8_t *uv_base = (uint8_t *)IOSurfaceGetBaseAddressOfPlane(frame_surface, 1);
                    size_t y_stride = IOSurfaceGetBytesPerRowOfPlane(frame_surface, 0);
                    size_t uv_stride = IOSurfaceGetBytesPerRowOfPlane(frame_surface, 1);
                    size_t y_width = IOSurfaceGetWidthOfPlane(frame_surface, 0);
                    size_t y_height = IOSurfaceGetHeightOfPlane(frame_surface, 0);
                    size_t uv_width = IOSurfaceGetWidthOfPlane(frame_surface, 1);
                    size_t uv_height = IOSurfaceGetHeightOfPlane(frame_surface, 1);

                    if (y_base != NULL && uv_base != NULL) {
                        callback(
                            pixel_format,
                            y_base,
                            y_stride * y_height,
                            y_width,
                            y_height,
                            y_stride,
                            uv_base,
                            uv_stride * uv_height,
                            uv_width,
                            uv_height,
                            uv_stride,
                            user_data);
                    }
                } else {
                    uint8_t *base = (uint8_t *)IOSurfaceGetBaseAddress(frame_surface);
                    size_t stride = IOSurfaceGetBytesPerRow(frame_surface);
                    size_t surface_width = IOSurfaceGetWidth(frame_surface);
                    size_t surface_height = IOSurfaceGetHeight(frame_surface);
                    size_t byte_count = stride * surface_height;

                    if (base != NULL && stride >= surface_width * 4) {
                        callback(
                            pixel_format,
                            base,
                            byte_count,
                            surface_width,
                            surface_height,
                            stride,
                            NULL,
                            0,
                            0,
                            0,
                            0,
                            user_data);
                    }
                }

                IOSurfaceUnlock(frame_surface, kIOSurfaceLockReadOnly, NULL);
            });

        if (g_stream == NULL || start_stream(g_stream) != kCGErrorSuccess) {
            g_last_error = "CGDisplayStream creation or start failed";
            if (g_stream != NULL) {
                CFRelease(g_stream);
                g_stream = NULL;
            }
            g_display = nil;
            g_queue = NULL;
            return 0;
        }

        g_last_error = "ok";
        return display_id;
    }
}

void t6_vd_stop(void) {
    @autoreleasepool {
        if (g_stream != NULL) {
            CGDisplayStreamStopFn stop_stream =
                (CGDisplayStreamStopFn)dlsym(RTLD_DEFAULT, "CGDisplayStreamStop");
            if (stop_stream != NULL) {
                stop_stream(g_stream);
            }
            CFRelease(g_stream);
            g_stream = NULL;
        }

        g_display = nil;
        g_queue = NULL;
    }
}
