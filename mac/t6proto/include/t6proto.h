#ifndef T6PROTO_H
#define T6PROTO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define T6_VENDOR_ID 0x0711u
#define T6_PRODUCT_ID_JUA365 0x5601u

#define T6_EP_BULK_OUT 0x02u
#define T6_EP_BULK_IN 0x81u
#define T6_EP_INTERRUPT_IN 0x83u

#define T6_SIGNATURE_DISPLAY 0x00u
#define T6_SIGNATURE_AUDIO 0x03u
#define T6_SIGNATURE_NV12DMA 0x07u

#define T6_PACKET_FLAG_NONE 0x00u
#define T6_PACKET_FLAG_CONTINUE_TO_SEND 0x01u

#define T6_DISPLAY_EXT_NONE 0x00u
#define T6_DISPLAY_EXT_FLIP_PRIMARY 0x03u
#define T6_DISPLAY_EXT_FLIP_SECONDARY 0x04u
#define T6_DISPLAY_EXT_CLIP_PRIMARY 0x05u
#define T6_DISPLAY_EXT_CLIP_SECONDARY 0x06u

#define T6_INT_FUNC_DISPLAY 0x04u
#define T6_INT_DISPLAY_EVENT_HDMI_CONNECT 0x01u
#define T6_INT_DISPLAY_EVENT_VGA_CONNECT 0x02u
#define T6_INT_DISPLAY_EVENT_FENCE_ID 0x04u
#define T6_INT_DISPLAY_EVENT_JPEG_ERROR 0x08u

#define T6_VIDEO_CMD_CLIP_PRIMARY 1u
#define T6_VIDEO_CMD_CLIP_SECONDARY 2u
#define T6_VIDEO_CMD_FLIP_PRIMARY 3u
#define T6_VIDEO_CMD_FLIP_SECONDARY 4u

#define T6_VIDEO_COLOR_YV12 4u
#define T6_VIDEO_COLOR_NV12 6u
#define T6_VIDEO_COLOR_RGB32 8u
#define T6_VIDEO_COLOR_RGB24 9u
#define T6_VIDEO_COLOR_YUV444 11u
#define T6_VIDEO_COLOR_JPEG 13u

#define T6_BULK_DMA_HEADER_SIZE 32u
#define T6_VIDEO_FLIP_HEADER_SIZE 48u
#define T6_INTERRUPT_PACKET_SIZE 64u

struct t6_bulk_dma_header {
	uint32_t signature;
	uint32_t payload_length;
	uint32_t payload_address;
	uint32_t packet_size;
	uint32_t packet_offset;
	uint8_t packet_flags;
	uint8_t packet_reserved[3];
	uint8_t function_specific[8];
};

struct t6_video_flip_header {
	uint32_t command;
	uint32_t payload_size;
	uint32_t fence_id;
	uint32_t target_format;
	uint16_t y_rgb_pitch;
	uint16_t uv_pitch;
	uint32_t y_rgb_data_fb_offset;
	uint32_t u_uv_data_offset;
	uint32_t v_data_offset;
	uint32_t source_format;
	uint8_t padding[11];
	uint8_t flags;
};

struct t6_display_interrupt {
	bool is_display;
	uint32_t display_data;
	uint8_t display_event;
	bool has_fence_id;
	bool has_jpeg_error;
};

void t6_write_bulk_dma_header(uint8_t out[T6_BULK_DMA_HEADER_SIZE],
			      const struct t6_bulk_dma_header *header);

void t6_build_display_bulk_header(uint8_t out[T6_BULK_DMA_HEADER_SIZE],
				  uint32_t payload_length,
				  uint32_t payload_address,
				  uint32_t packet_size,
				  uint32_t packet_offset,
				  bool more_packets);

void t6_write_video_flip_header(uint8_t out[T6_VIDEO_FLIP_HEADER_SIZE],
				const struct t6_video_flip_header *header);

struct t6_video_flip_header
t6_make_jpeg_flip_header(uint8_t display_index, uint32_t payload_size,
			 uint16_t width, uint16_t height,
			 uint32_t command_address, uint32_t framebuffer_address,
			 uint8_t flags);

struct t6_video_flip_header
t6_make_nv12_flip_header(uint8_t display_index, uint32_t payload_size,
			 uint16_t width, uint16_t height,
			 uint32_t framebuffer_address, uint8_t flags);

struct t6_display_interrupt
t6_parse_display_interrupt(const uint8_t packet[T6_INTERRUPT_PACKET_SIZE]);

size_t t6_fragment_count(uint32_t payload_length, uint32_t max_packet_size);

uint32_t t6_fragment_size(uint32_t payload_length, uint32_t max_packet_size,
			  size_t fragment_index);

uint32_t t6_fragment_offset(uint32_t max_packet_size, size_t fragment_index);

bool t6_fragment_has_more(uint32_t payload_length, uint32_t max_packet_size,
			  size_t fragment_index);

#endif

