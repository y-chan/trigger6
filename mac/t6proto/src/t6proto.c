#include "t6proto.h"

#include <string.h>

static void write_le16(uint8_t *out, uint16_t value)
{
	out[0] = (uint8_t)(value & 0xffu);
	out[1] = (uint8_t)((value >> 8) & 0xffu);
}

static void write_le32(uint8_t *out, uint32_t value)
{
	out[0] = (uint8_t)(value & 0xffu);
	out[1] = (uint8_t)((value >> 8) & 0xffu);
	out[2] = (uint8_t)((value >> 16) & 0xffu);
	out[3] = (uint8_t)((value >> 24) & 0xffu);
}

static uint32_t read_le32(const uint8_t *in)
{
	return (uint32_t)in[0] | ((uint32_t)in[1] << 8) |
	       ((uint32_t)in[2] << 16) | ((uint32_t)in[3] << 24);
}

static uint16_t align_u16(uint16_t value, uint16_t alignment)
{
	return (uint16_t)(((uint32_t)value + alignment - 1u) / alignment *
			  alignment);
}

void t6_write_bulk_dma_header(uint8_t out[T6_BULK_DMA_HEADER_SIZE],
			      const struct t6_bulk_dma_header *header)
{
	memset(out, 0, T6_BULK_DMA_HEADER_SIZE);
	write_le32(out + 0, header->signature);
	write_le32(out + 4, header->payload_length);
	write_le32(out + 8, header->payload_address);
	write_le32(out + 12, header->packet_size);
	write_le32(out + 16, header->packet_offset);
	out[20] = header->packet_flags;
	memcpy(out + 24, header->function_specific,
	       sizeof(header->function_specific));
}

void t6_build_display_bulk_header(uint8_t out[T6_BULK_DMA_HEADER_SIZE],
				  uint32_t payload_length,
				  uint32_t payload_address,
				  uint32_t packet_size,
				  uint32_t packet_offset,
				  bool more_packets)
{
	struct t6_bulk_dma_header header;

	memset(&header, 0, sizeof(header));
	header.signature = T6_SIGNATURE_DISPLAY;
	header.payload_length = payload_length;
	header.payload_address = payload_address;
	header.packet_size = packet_size;
	header.packet_offset = packet_offset;
	header.packet_flags = more_packets ? T6_PACKET_FLAG_CONTINUE_TO_SEND :
					     T6_PACKET_FLAG_NONE;

	t6_write_bulk_dma_header(out, &header);
}

void t6_write_video_flip_header(uint8_t out[T6_VIDEO_FLIP_HEADER_SIZE],
				const struct t6_video_flip_header *header)
{
	memset(out, 0, T6_VIDEO_FLIP_HEADER_SIZE);
	write_le32(out + 0, header->command);
	write_le32(out + 4, header->payload_size);
	write_le32(out + 8, header->fence_id);
	write_le32(out + 12, header->target_format);
	write_le16(out + 16, header->y_rgb_pitch);
	write_le16(out + 18, header->uv_pitch);
	write_le32(out + 20, header->y_rgb_data_fb_offset);
	write_le32(out + 24, header->u_uv_data_offset);
	write_le32(out + 28, header->v_data_offset);
	write_le32(out + 32, header->source_format);
	out[47] = header->flags;
}

struct t6_video_flip_header
t6_make_jpeg_flip_header(uint8_t display_index, uint32_t payload_size,
			 uint16_t width, uint16_t height,
			 uint32_t command_address, uint32_t framebuffer_address,
			 uint8_t flags)
{
	struct t6_video_flip_header header;
	uint16_t pitch = align_u16(width, 32);
	uint32_t y_block_size = (uint32_t)pitch * align_u16(height, 32) + 1024u;

	(void)command_address;

	memset(&header, 0, sizeof(header));
	header.command = display_index == 0 ? T6_VIDEO_CMD_FLIP_PRIMARY :
					      T6_VIDEO_CMD_FLIP_SECONDARY;
	header.payload_size = payload_size;
	header.target_format = T6_VIDEO_COLOR_NV12;
	header.y_rgb_pitch = pitch;
	header.uv_pitch = pitch;
	header.y_rgb_data_fb_offset = framebuffer_address;
	header.u_uv_data_offset = framebuffer_address + y_block_size;
	header.source_format = T6_VIDEO_COLOR_JPEG;
	header.flags = flags;
	return header;
}

struct t6_video_flip_header
t6_make_nv12_flip_header(uint8_t display_index, uint32_t payload_size,
			 uint16_t width, uint16_t height,
			 uint32_t framebuffer_address, uint8_t flags)
{
	struct t6_video_flip_header header;
	uint16_t pitch = align_u16(width, 16);
	uint32_t uv_offset = (uint32_t)pitch * height;

	memset(&header, 0, sizeof(header));
	header.command = display_index == 0 ? T6_VIDEO_CMD_FLIP_PRIMARY :
					      T6_VIDEO_CMD_FLIP_SECONDARY;
	header.payload_size = payload_size;
	header.target_format = T6_VIDEO_COLOR_NV12;
	header.y_rgb_pitch = pitch;
	header.uv_pitch = pitch;
	header.y_rgb_data_fb_offset = framebuffer_address +
				      T6_VIDEO_FLIP_HEADER_SIZE;
	header.u_uv_data_offset = header.y_rgb_data_fb_offset + uv_offset;
	header.source_format = T6_VIDEO_COLOR_NV12;
	header.flags = flags;
	return header;
}

struct t6_display_interrupt
t6_parse_display_interrupt(const uint8_t packet[T6_INTERRUPT_PACKET_SIZE])
{
	struct t6_display_interrupt result;

	memset(&result, 0, sizeof(result));
	result.is_display = (packet[0] & T6_INT_FUNC_DISPLAY) != 0;
	result.display_data = read_le32(packet + 12);
	result.display_event = packet[19];
	result.has_fence_id =
		result.is_display &&
		((result.display_event & T6_INT_DISPLAY_EVENT_FENCE_ID) != 0);
	result.has_jpeg_error =
		result.is_display &&
		((result.display_event & T6_INT_DISPLAY_EVENT_JPEG_ERROR) != 0);
	return result;
}

size_t t6_fragment_count(uint32_t payload_length, uint32_t max_packet_size)
{
	if (payload_length == 0 || max_packet_size == 0)
		return 0;
	return (payload_length + max_packet_size - 1u) / max_packet_size;
}

uint32_t t6_fragment_size(uint32_t payload_length, uint32_t max_packet_size,
			  size_t fragment_index)
{
	uint32_t offset = t6_fragment_offset(max_packet_size, fragment_index);
	uint32_t remaining;

	if (max_packet_size == 0 || offset >= payload_length)
		return 0;

	remaining = payload_length - offset;
	return remaining < max_packet_size ? remaining : max_packet_size;
}

uint32_t t6_fragment_offset(uint32_t max_packet_size, size_t fragment_index)
{
	return max_packet_size * (uint32_t)fragment_index;
}

bool t6_fragment_has_more(uint32_t payload_length, uint32_t max_packet_size,
			  size_t fragment_index)
{
	uint32_t offset = t6_fragment_offset(max_packet_size, fragment_index);
	uint32_t size =
		t6_fragment_size(payload_length, max_packet_size, fragment_index);

	return size != 0 && offset + size < payload_length;
}

