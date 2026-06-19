#ifndef EVDI_T6_H
#define EVDI_T6_H
#include <stdint.h>
#include <libusb-1.0/libusb.h>
#include <evdi_lib.h>
#include "queue.h"

#define MAX_DIRTS 16
#define MAX_T6_DEVICES	16

#define DEBUG
#ifdef DEBUG
#define DEBUG_PRINT(fmt, args...)    fprintf(stderr, fmt, ## args)
#else
#define DEBUG_PRINT(fmt, args...)    /* Don't do anything in release builds */
#endif
#include "simclist.h"


typedef unsigned int   UINT32;
typedef unsigned short UINT16;
typedef unsigned char  UINT8;
//#pragma pack(1)



typedef struct T6evdi{
	libusb_device_handle*	t6usbdev;
	evdi_handle             ev_handle; 
	int                     display_id; 
	int                     usb_bus_id;
	int                     usb_dev_id;
	int                     image_work_process;
	int                     jpg_work_process;
	int                     audio_work_process;
	int                     cursor_work_process;
	int                     event_process;
	int                     usb_process;
	int                     ramsize;
	int                     disp_set_mode;
	int                     fbAddr ;
	int                     cmdAddr;  
	int                     frameupdate ;
	int                     audio_only ;
	int                     interface_num;
	int                     jpg_reset_fun;
	int                     jpg_rotate ;
	UINT16      			Width;
	UINT16					Height;
	UINT8					disp_interface;
	UINT8					edid[512];
	UINT8                   *video_buffer;
	UINT8					bRun4K30;
	UINT8					dispcaps; // 0-3 bits: disp1, 4-7bits: disp2;
									   // D0: DAC (Internal VGA)
									   // D1: DVO (External HDMI)
									   // D2: DVI (Internal DVI)
									   // D3: LVDS (Internal HDMI)
	UINT8					*detach_all_event;
	UINT32					pixelformat;
	//queue_t*                audio_queue;
	queue_t*                jpg_queue;
	queue_t*                cursor_queue;
	queue_t*                cursor_queue_pos;
	list_t                  jpg_list_queue;
	struct evdi_box*        evdi_list_queue;
	pthread_mutex_t         *lock; 				//for usb bulk endpoint
	pthread_mutex_t         *usbctrl_lock; 		//for usb ctrl endpoint
	pthread_mutex_t         *image_mutex;
	pthread_mutex_t         bulkusb_mutex;
	struct T6evdi *next;
}T6EVDI, *PT6EVDI;


struct jpg_packet{
	unsigned long jpgImageSize;
	char *buffer ; 
};

struct int_proc_para{
	libusb_device_handle*	t6usbdev;
	pthread_mutex_t         *lock;
	unsigned char			*detach_all_event;
};


struct evdi_box{
  char box[128];
};


#define fourcc_code(a, b, c, d) ((__u32)(a) | ((__u32)(b) << 8) | \
				 ((__u32)(c) << 16) | ((__u32)(d) << 24))
				 
#define DRM_FORMAT_XRGB8888	fourcc_code('X', 'R', '2', '4') /* [31:0] x:R:G:B 8:8:8:8 little endian */
#define DRM_FORMAT_XBGR8888	fourcc_code('X', 'B', '2', '4') /* [31:0] x:B:G:R 8:8:8:8 little endian */
#define DRM_FORMAT_ARGB8888	fourcc_code('A', 'R', '2', '4') /* [31:0] A:R:G:B 8:8:8:8 little endian */
#define DRM_FORMAT_ABGR8888	fourcc_code('A', 'B', '2', '4') /* [31:0] A:B:G:R 8:8:8:8 little endian */
//#pragma pack()

#endif



