#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <VideoToolbox/VideoToolbox.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void print_fourcc(uint32_t value) {
    char code[5];
    code[0] = (char)((value >> 24) & 0xff);
    code[1] = (char)((value >> 16) & 0xff);
    code[2] = (char)((value >> 8) & 0xff);
    code[3] = (char)(value & 0xff);
    code[4] = 0;
    printf("%s", code);
}

static uint16_t read_be16(const uint8_t *p) {
    return (uint16_t)(((uint16_t)p[0] << 8) | (uint16_t)p[1]);
}

static void print_jpeg_info(const uint8_t *data, size_t len) {
    if (len < 4 || data[0] != 0xff || data[1] != 0xd8) {
        printf("JPEG info: not a JPEG codestream len=%zu\n", len);
        return;
    }

    size_t offset = 2;
    while (offset + 4 <= len) {
        while (offset < len && data[offset] == 0xff) {
            offset++;
        }
        if (offset >= len) {
            break;
        }

        uint8_t marker = data[offset++];
        if (marker == 0xd9 || marker == 0xda) {
            break;
        }
        if (offset + 2 > len) {
            break;
        }

        uint16_t segment_len = read_be16(data + offset);
        if (segment_len < 2 || offset + segment_len > len) {
            break;
        }

        const uint8_t *segment = data + offset + 2;
        size_t payload_len = segment_len - 2;
        if ((marker == 0xc0 || marker == 0xc2) && payload_len >= 6) {
            uint16_t height = read_be16(segment + 1);
            uint16_t width = read_be16(segment + 3);
            uint8_t components = segment[5];
            printf("JPEG info: marker=0x%02x progressive=%s width=%u height=%u components=%u sampling=",
                   marker,
                   marker == 0xc2 ? "true" : "false",
                   width,
                   height,
                   components);
            for (uint8_t i = 0; i < components; i++) {
                size_t component_offset = 6 + (size_t)i * 3;
                if (component_offset + 2 >= payload_len) {
                    break;
                }
                uint8_t id = segment[component_offset];
                uint8_t hv = segment[component_offset + 1];
                printf("%sid%u:%ux%u", i == 0 ? "" : ",", id, hv >> 4, hv & 0x0f);
            }
            printf("\n");
            return;
        }

        offset += segment_len;
    }

    printf("JPEG info: SOF0/SOF2 marker not found len=%zu\n", len);
}

static void fill_test_pattern(CVPixelBufferRef pixel_buffer, uint32_t frame_index) {
    CVPixelBufferLockBaseAddress(pixel_buffer, 0);
    uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixel_buffer);
    size_t stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
    size_t width = CVPixelBufferGetWidth(pixel_buffer);
    size_t height = CVPixelBufferGetHeight(pixel_buffer);

    for (size_t y = 0; y < height; y++) {
        uint8_t *row = base + y * stride;
        for (size_t x = 0; x < width; x++) {
            uint8_t *px = row + x * 4;
            uint8_t r = (uint8_t)((x + frame_index * 3) & 0xff);
            uint8_t g = (uint8_t)((y + frame_index * 5) & 0xff);
            uint8_t b = (uint8_t)(((x ^ y) + frame_index * 7) & 0xff);

            if (x < 80 || y < 80) {
                r = 235;
                g = 40;
                b = 45;
            } else if (((x / 40) + (y / 40) + frame_index) % 2 == 0) {
                r = 245;
                g = 245;
                b = 245;
            }

            px[0] = b;
            px[1] = g;
            px[2] = r;
            px[3] = 255;
        }
    }

    CVPixelBufferUnlockBaseAddress(pixel_buffer, 0);
}

static double now_ms(void) {
    return CFAbsoluteTimeGetCurrent() * 1000.0;
}

typedef struct T6VTEncodeResult {
    dispatch_semaphore_t semaphore;
    OSStatus status;
    CMSampleBufferRef sample_buffer;
} T6VTEncodeResult;

typedef struct T6VTJpegEncoder {
    VTCompressionSessionRef session;
    CVPixelBufferRef pixel_buffer;
    uint8_t *jpeg_data;
    size_t jpeg_len;
    size_t jpeg_capacity;
    size_t width;
    size_t height;
    float quality;
} T6VTJpegEncoder;

static NSString *preferred_jpeg_encoder_id(void);

static void t6_vt_jpeg_output_callback(void *output_callback_refcon,
                                       void *source_frame_refcon,
                                       OSStatus status,
                                       VTEncodeInfoFlags info_flags,
                                       CMSampleBufferRef sample_buffer) {
    (void)output_callback_refcon;
    (void)info_flags;
    T6VTEncodeResult *result = (T6VTEncodeResult *)source_frame_refcon;
    if (result == NULL) {
        return;
    }

    result->status = status;
    if (sample_buffer != NULL) {
        result->sample_buffer = (CMSampleBufferRef)CFRetain(sample_buffer);
    }
    dispatch_semaphore_signal(result->semaphore);
}

static int t6_vt_jpeg_set_quality(T6VTJpegEncoder *encoder, float quality) {
    if (encoder == NULL || encoder->session == NULL) {
        return -1;
    }
    if (quality < 0.0f) {
        quality = 0.0f;
    } else if (quality > 1.0f) {
        quality = 1.0f;
    }
    if (encoder->quality == quality) {
        return 0;
    }
    CFNumberRef quality_number =
        CFNumberCreate(kCFAllocatorDefault, kCFNumberFloatType, &quality);
    if (quality_number == NULL) {
        return -2;
    }
    OSStatus status =
        VTSessionSetProperty(encoder->session, kVTCompressionPropertyKey_Quality, quality_number);
    CFRelease(quality_number);
    if (status != noErr) {
        return (int)status;
    }
    encoder->quality = quality;
    return 0;
}

void *t6_vt_jpeg_encoder_create(size_t width, size_t height, float quality) {
    @autoreleasepool {
        T6VTJpegEncoder *encoder = (T6VTJpegEncoder *)calloc(1, sizeof(T6VTJpegEncoder));
        if (encoder == NULL) {
            return NULL;
        }
        encoder->width = width;
        encoder->height = height;
        encoder->quality = -1.0f;

        NSDictionary *pixel_attrs = @{
            (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
            (__bridge NSString *)kCVPixelBufferWidthKey : @(width),
            (__bridge NSString *)kCVPixelBufferHeightKey : @(height),
            (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey : @{},
        };
        CVReturn cv_status = CVPixelBufferCreate(kCFAllocatorDefault,
                                                 width,
                                                 height,
                                                 kCVPixelFormatType_32BGRA,
                                                 NULL,
                                                 &encoder->pixel_buffer);
        if (cv_status != kCVReturnSuccess || encoder->pixel_buffer == NULL) {
            fprintf(stderr, "t6_vt_jpeg_encoder_create: CVPixelBufferCreate failed: %d\n", (int)cv_status);
            free(encoder);
            return NULL;
        }

        NSString *encoder_id = preferred_jpeg_encoder_id();
        NSDictionary *encoder_spec = nil;
        if (encoder_id != nil) {
            encoder_spec = @{
                (__bridge NSString *)kVTVideoEncoderSpecification_EncoderID : encoder_id,
                (__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder : @YES,
            };
        }

        OSStatus status =
            VTCompressionSessionCreate(kCFAllocatorDefault,
                                       (int32_t)width,
                                       (int32_t)height,
                                       kCMVideoCodecType_JPEG,
                                       encoder_spec == nil ? NULL : (__bridge CFDictionaryRef)encoder_spec,
                                       (__bridge CFDictionaryRef)pixel_attrs,
                                       NULL,
                                       t6_vt_jpeg_output_callback,
                                       NULL,
                                       &encoder->session);
        if (status != noErr || encoder->session == NULL) {
            status = VTCompressionSessionCreate(kCFAllocatorDefault,
                                                (int32_t)width,
                                                (int32_t)height,
                                                kCMVideoCodecType_JPEG,
                                                NULL,
                                                (__bridge CFDictionaryRef)pixel_attrs,
                                                NULL,
                                                t6_vt_jpeg_output_callback,
                                                NULL,
                                                &encoder->session);
        }
        if (status != noErr || encoder->session == NULL) {
            fprintf(stderr, "t6_vt_jpeg_encoder_create: VTCompressionSessionCreate failed: %d\n", (int)status);
            CVPixelBufferRelease(encoder->pixel_buffer);
            free(encoder);
            return NULL;
        }

        t6_vt_jpeg_set_quality(encoder, quality);
        VTSessionSetProperty(encoder->session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
        VTSessionSetProperty(encoder->session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
        int max_delay = 0;
        CFNumberRef max_delay_number =
            CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &max_delay);
        if (max_delay_number != NULL) {
            VTSessionSetProperty(encoder->session,
                                 kVTCompressionPropertyKey_MaxFrameDelayCount,
                                 max_delay_number);
            CFRelease(max_delay_number);
        }
        if (@available(macOS 11.0, *)) {
            VTSessionSetProperty(encoder->session,
                                 kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                                 kCFBooleanTrue);
        }
        status = VTCompressionSessionPrepareToEncodeFrames(encoder->session);
        if (status != noErr) {
            fprintf(stderr, "t6_vt_jpeg_encoder_create: VTCompressionSessionPrepareToEncodeFrames failed: %d\n", (int)status);
            VTCompressionSessionInvalidate(encoder->session);
            CFRelease(encoder->session);
            CVPixelBufferRelease(encoder->pixel_buffer);
            free(encoder);
            return NULL;
        }

        return encoder;
    }
}

void t6_vt_jpeg_encoder_destroy(void *opaque) {
    T6VTJpegEncoder *encoder = (T6VTJpegEncoder *)opaque;
    if (encoder == NULL) {
        return;
    }
    if (encoder->session != NULL) {
        VTCompressionSessionCompleteFrames(encoder->session, kCMTimeInvalid);
        VTCompressionSessionInvalidate(encoder->session);
        CFRelease(encoder->session);
    }
    if (encoder->pixel_buffer != NULL) {
        CVPixelBufferRelease(encoder->pixel_buffer);
    }
    free(encoder->jpeg_data);
    free(encoder);
}

int t6_vt_jpeg_encoder_encode_bgra(void *opaque,
                                   const uint8_t *bgra,
                                   size_t width,
                                   size_t height,
                                   size_t stride,
                                   float quality,
                                   const uint8_t **jpeg_data,
                                   size_t *jpeg_len) {
    @autoreleasepool {
        T6VTJpegEncoder *encoder = (T6VTJpegEncoder *)opaque;
        if (encoder == NULL || bgra == NULL || jpeg_data == NULL || jpeg_len == NULL) {
            return -1;
        }
        if (width != encoder->width || height != encoder->height) {
            return -2;
        }
        int quality_status = t6_vt_jpeg_set_quality(encoder, quality);
        if (quality_status != 0) {
            return quality_status;
        }

        CVPixelBufferLockBaseAddress(encoder->pixel_buffer, 0);
        uint8_t *dst_base = (uint8_t *)CVPixelBufferGetBaseAddress(encoder->pixel_buffer);
        size_t dst_stride = CVPixelBufferGetBytesPerRow(encoder->pixel_buffer);
        for (size_t y = 0; y < height; y++) {
            memcpy(dst_base + y * dst_stride, bgra + y * stride, width * 4);
        }
        CVPixelBufferUnlockBaseAddress(encoder->pixel_buffer, 0);

        T6VTEncodeResult result;
        result.semaphore = dispatch_semaphore_create(0);
        result.status = noErr;
        result.sample_buffer = NULL;

        VTEncodeInfoFlags info_flags = 0;
        OSStatus status = VTCompressionSessionEncodeFrame(encoder->session,
                                                          encoder->pixel_buffer,
                                                          CMTimeMake(0, 60),
                                                          kCMTimeInvalid,
                                                          NULL,
                                                          &result,
                                                          &info_flags);
        if (status != noErr) {
            return (int)status;
        }
        long wait_status =
            dispatch_semaphore_wait(result.semaphore,
                                    dispatch_time(DISPATCH_TIME_NOW, 5LL * NSEC_PER_SEC));
        if (wait_status != 0) {
            if (result.sample_buffer != NULL) {
                CFRelease(result.sample_buffer);
            }
            return -3;
        }
        if (result.status != noErr || result.sample_buffer == NULL) {
            if (result.sample_buffer != NULL) {
                CFRelease(result.sample_buffer);
            }
            return result.status == noErr ? -4 : (int)result.status;
        }

        CMBlockBufferRef block_buffer = CMSampleBufferGetDataBuffer(result.sample_buffer);
        size_t len = block_buffer == NULL ? 0 : CMBlockBufferGetDataLength(block_buffer);
        if (block_buffer == NULL || len == 0) {
            CFRelease(result.sample_buffer);
            return -5;
        }
        if (encoder->jpeg_capacity < len) {
            uint8_t *new_data = (uint8_t *)realloc(encoder->jpeg_data, len);
            if (new_data == NULL) {
                CFRelease(result.sample_buffer);
                return -6;
            }
            encoder->jpeg_data = new_data;
            encoder->jpeg_capacity = len;
        }
        status = CMBlockBufferCopyDataBytes(block_buffer, 0, len, encoder->jpeg_data);
        CFRelease(result.sample_buffer);
        if (status != noErr) {
            return (int)status;
        }
        encoder->jpeg_len = len;
        *jpeg_data = encoder->jpeg_data;
        *jpeg_len = encoder->jpeg_len;
        return 0;
    }
}

static void print_encoder_list(void) {
    CFArrayRef encoders_ref = NULL;
    OSStatus status = VTCopyVideoEncoderList(NULL, &encoders_ref);
    if (status != noErr || encoders_ref == NULL) {
        printf("VTCopyVideoEncoderList failed: %d\n", (int)status);
        return;
    }

    NSArray *encoders = CFBridgingRelease(encoders_ref);
    NSUInteger jpeg_count = 0;
    printf("VideoToolbox JPEG encoders:\n");

    for (NSDictionary *encoder in encoders) {
        NSNumber *codec_number = encoder[(__bridge NSString *)kVTVideoEncoderList_CodecType];
        if (codec_number == nil || [codec_number unsignedIntValue] != kCMVideoCodecType_JPEG) {
            continue;
        }

        jpeg_count++;
        NSString *encoder_id = encoder[(__bridge NSString *)kVTVideoEncoderList_EncoderID] ?: @"-";
        NSString *codec_name = encoder[(__bridge NSString *)kVTVideoEncoderList_CodecName] ?: @"-";
        NSString *encoder_name = encoder[(__bridge NSString *)kVTVideoEncoderList_EncoderName] ?: @"-";
        NSString *display_name = encoder[(__bridge NSString *)kVTVideoEncoderList_DisplayName] ?: @"-";
        NSNumber *hardware = encoder[(__bridge NSString *)kVTVideoEncoderList_IsHardwareAccelerated];

        printf("  codec=");
        print_fourcc([codec_number unsignedIntValue]);
        printf(" encoder_id=%s codec_name=%s encoder_name=%s display_name=%s hardware=%s\n",
               [encoder_id UTF8String],
               [codec_name UTF8String],
               [encoder_name UTF8String],
               [display_name UTF8String],
               hardware == nil ? "unknown" : ([hardware boolValue] ? "true" : "false"));
    }

    if (jpeg_count == 0) {
        printf("  none\n");
    }
}

static NSString *preferred_jpeg_encoder_id(void) {
    CFArrayRef encoders_ref = NULL;
    OSStatus status = VTCopyVideoEncoderList(NULL, &encoders_ref);
    if (status != noErr || encoders_ref == NULL) {
        return nil;
    }

    NSArray *encoders = CFBridgingRelease(encoders_ref);
    NSString *fallback_encoder_id = nil;
    for (NSDictionary *encoder in encoders) {
        NSNumber *codec_number = encoder[(__bridge NSString *)kVTVideoEncoderList_CodecType];
        if (codec_number == nil || [codec_number unsignedIntValue] != kCMVideoCodecType_JPEG) {
            continue;
        }

        NSString *encoder_id = encoder[(__bridge NSString *)kVTVideoEncoderList_EncoderID];
        if (encoder_id == nil) {
            continue;
        }

        NSNumber *hardware = encoder[(__bridge NSString *)kVTVideoEncoderList_IsHardwareAccelerated];
        if (hardware != nil && [hardware boolValue]) {
            return encoder_id;
        }
        if (fallback_encoder_id == nil) {
            fallback_encoder_id = encoder_id;
        }
    }

    return fallback_encoder_id;
}

static void print_session_encoder_info(VTCompressionSessionRef session) {
    CFTypeRef encoder_id_ref = NULL;
    OSStatus encoder_id_status = VTSessionCopyProperty(session,
                                                       kVTCompressionPropertyKey_EncoderID,
                                                       kCFAllocatorDefault,
                                                       &encoder_id_ref);
    CFTypeRef hardware_ref = NULL;
    OSStatus hardware_status =
        VTSessionCopyProperty(session,
                              kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                              kCFAllocatorDefault,
                              &hardware_ref);

    NSString *encoder_id = encoder_id_ref == NULL ? @"-" : (__bridge NSString *)encoder_id_ref;
    const char *hardware = "unknown";
    if (hardware_status == noErr && hardware_ref != NULL) {
        hardware = CFBooleanGetValue((CFBooleanRef)hardware_ref) ? "true" : "false";
    }
    printf("VT session encoder: encoder_id=%s hardware=%s",
           encoder_id_status == noErr ? [encoder_id UTF8String] : "-",
           hardware);
    if (encoder_id_status != noErr) {
        printf(" encoder_id_status=%d", (int)encoder_id_status);
    }
    if (hardware_status != noErr) {
        printf(" hardware_status=%d", (int)hardware_status);
    }
    printf("\n");

    if (encoder_id_ref != NULL) {
        CFRelease(encoder_id_ref);
    }
    if (hardware_ref != NULL) {
        CFRelease(hardware_ref);
    }
}

int t6_vt_jpeg_probe(size_t width, size_t height, double quality, uint32_t frames) {
    @autoreleasepool {
        print_encoder_list();

        NSDictionary *pixel_attrs = @{
            (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
            (__bridge NSString *)kCVPixelBufferWidthKey : @(width),
            (__bridge NSString *)kCVPixelBufferHeightKey : @(height),
            (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey : @{},
        };
        CVPixelBufferRef pixel_buffer = NULL;
        CVReturn cv_status = CVPixelBufferCreate(kCFAllocatorDefault,
                                                 width,
                                                 height,
                                                 kCVPixelFormatType_32BGRA,
                                                 NULL,
                                                 &pixel_buffer);
        if (cv_status != kCVReturnSuccess || pixel_buffer == NULL) {
            printf("CVPixelBufferCreate failed: %d\n", (int)cv_status);
            return 2;
        }

        VTCompressionSessionRef session = NULL;
        NSString *encoder_id = preferred_jpeg_encoder_id();
        NSDictionary *encoder_spec = nil;
        if (encoder_id != nil) {
            encoder_spec = @{
                (__bridge NSString *)kVTVideoEncoderSpecification_EncoderID : encoder_id,
                (__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder : @YES,
            };
            printf("Creating JPEG session with preferred encoder_id=%s\n", [encoder_id UTF8String]);
        }

        OSStatus status =
            VTCompressionSessionCreate(kCFAllocatorDefault,
                                       (int32_t)width,
                                       (int32_t)height,
                                       kCMVideoCodecType_JPEG,
                                       encoder_spec == nil ? NULL : (__bridge CFDictionaryRef)encoder_spec,
                                       (__bridge CFDictionaryRef)pixel_attrs,
                                       NULL,
                                       t6_vt_jpeg_output_callback,
                                       NULL,
                                       &session);
        if (status != noErr || session == NULL) {
            printf("Retrying JPEG session with automatic encoder selection\n");
            status = VTCompressionSessionCreate(kCFAllocatorDefault,
                                                (int32_t)width,
                                                (int32_t)height,
                                                kCMVideoCodecType_JPEG,
                                                NULL,
                                                (__bridge CFDictionaryRef)pixel_attrs,
                                                NULL,
                                                t6_vt_jpeg_output_callback,
                                                NULL,
                                                &session);
        }
        if (status != noErr || session == NULL) {
            printf("VTCompressionSessionCreate(JPEG) failed: %d\n", (int)status);
            CVPixelBufferRelease(pixel_buffer);
            return 3;
        }

        float quality_f = (float)quality;
        CFNumberRef quality_number =
            CFNumberCreate(kCFAllocatorDefault, kCFNumberFloatType, &quality_f);
        VTSessionSetProperty(session, kVTCompressionPropertyKey_Quality, quality_number);
        CFRelease(quality_number);
        VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
        if (@available(macOS 11.0, *)) {
            VTSessionSetProperty(session,
                                 kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                                 kCFBooleanTrue);
        }

        status = VTCompressionSessionPrepareToEncodeFrames(session);
        if (status != noErr) {
            printf("VTCompressionSessionPrepareToEncodeFrames failed: %d\n", (int)status);
            VTCompressionSessionInvalidate(session);
            CFRelease(session);
            CVPixelBufferRelease(pixel_buffer);
            return 4;
        }

        printf("VT JPEG session: width=%zu height=%zu quality=%.3f frames=%u\n",
               width,
               height,
               quality,
               frames);
        print_session_encoder_info(session);

        double total_encode_ms = 0.0;
        double max_encode_ms = 0.0;
        size_t first_len = 0;
        uint8_t *first_data = NULL;

        for (uint32_t frame = 0; frame < frames; frame++) {
            fill_test_pattern(pixel_buffer, frame);

            T6VTEncodeResult result;
            result.semaphore = dispatch_semaphore_create(0);
            result.status = noErr;
            result.sample_buffer = NULL;

            double start_ms = now_ms();
            VTEncodeInfoFlags info_flags = 0;
            status = VTCompressionSessionEncodeFrame(
                session,
                pixel_buffer,
                CMTimeMake(frame, 60),
                kCMTimeInvalid,
                NULL,
                &result,
                &info_flags);

            if (status != noErr) {
                printf("VTCompressionSessionEncodeFrame failed at frame %u: %d\n",
                       frame,
                       (int)status);
                if (result.sample_buffer != NULL) {
                    CFRelease(result.sample_buffer);
                }
                VTCompressionSessionInvalidate(session);
                CFRelease(session);
                CVPixelBufferRelease(pixel_buffer);
                return 5;
            }

            long wait_status =
                dispatch_semaphore_wait(result.semaphore,
                                        dispatch_time(DISPATCH_TIME_NOW, 5LL * NSEC_PER_SEC));
            double elapsed_ms = now_ms() - start_ms;
            if (elapsed_ms > max_encode_ms) {
                max_encode_ms = elapsed_ms;
            }
            total_encode_ms += elapsed_ms;

            if (wait_status != 0) {
                printf("VT JPEG encode timed out at frame %u\n", frame);
                if (result.sample_buffer != NULL) {
                    CFRelease(result.sample_buffer);
                }
                VTCompressionSessionInvalidate(session);
                CFRelease(session);
                CVPixelBufferRelease(pixel_buffer);
                return 6;
            }
            if (result.status != noErr || result.sample_buffer == NULL) {
                printf("VT JPEG output failed at frame %u: %d sample=%s\n",
                       frame,
                       (int)result.status,
                       result.sample_buffer == NULL ? "null" : "present");
                if (result.sample_buffer != NULL) {
                    CFRelease(result.sample_buffer);
                }
                VTCompressionSessionInvalidate(session);
                CFRelease(session);
                CVPixelBufferRelease(pixel_buffer);
                return 7;
            }

            CMBlockBufferRef block_buffer = CMSampleBufferGetDataBuffer(result.sample_buffer);
            size_t len = block_buffer == NULL ? 0 : CMBlockBufferGetDataLength(block_buffer);
            if (frame == 0 && block_buffer != NULL && len > 0) {
                first_len = len;
                first_data = (uint8_t *)malloc(first_len);
                if (first_data != NULL) {
                    OSStatus copy_status =
                        CMBlockBufferCopyDataBytes(block_buffer, 0, first_len, first_data);
                    if (copy_status != noErr) {
                        free(first_data);
                        first_data = NULL;
                        first_len = 0;
                    }
                }
            }

            CFRelease(result.sample_buffer);
        }

        if (first_data != NULL) {
            printf("First JPEG payload bytes: %zu\n", first_len);
            print_jpeg_info(first_data, first_len);
            NSData *data = [NSData dataWithBytes:first_data length:first_len];
            NSString *path = @"/tmp/t6-vt-jpeg-probe.jpg";
            if ([data writeToFile:path atomically:YES]) {
                printf("Wrote %s\n", [path UTF8String]);
            }
            free(first_data);
        } else {
            printf("No JPEG payload captured from first frame\n");
        }

        printf("VT JPEG encode profile: frames=%u avg_ms=%.2f max_ms=%.2f\n",
               frames,
               total_encode_ms / (double)frames,
               max_encode_ms);

        VTCompressionSessionCompleteFrames(session, kCMTimeInvalid);
        VTCompressionSessionInvalidate(session);
        CFRelease(session);
        CVPixelBufferRelease(pixel_buffer);
        return 0;
    }
}
