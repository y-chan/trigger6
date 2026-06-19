#ifndef _T6USBDONGLE_H_
#define _T6USBDONGLE_H_

#include <libusb-1.0/libusb.h>
#include "t6.h"
#include "t6auddef.h"
#include "t6bulkdef.h"
#include "evdi_t6.h"

unsigned long GetTickCount();
void hex_dump(char *data, int size, char *caption);
void t6_save_file(char* p,int size);

int  t6_libusb_get_displaysectionheader(libusb_device_handle *t6usbdev, UINT8 *dispcap);
int  t6_libusb_get_jpegreset(libusb_device_handle *t6usbdev);
int  t6_libusb_get_rotate(libusb_device_handle *t6usbdev);

int  t6_libusb_get_ram_size(PT6EVDI t6dev);

int  t6_libusb_set_monitor_power(PT6EVDI t6dev,char on);
void t6_libusb_donglereset(PT6EVDI t6dev);
int  t6_libusb_get_edid(PT6EVDI t6dev );
int  t6_libusb_set_AudioEngineStatus(PT6EVDI t6dev );
int  t6_libusb_set_softready(PT6EVDI t6dev);
int  t6_libusb_set_resolution(PT6EVDI t6dev, int w,int h, int fps);
int  t6_libusb_Rgb24_full_block(PT6EVDI t6dev ,int fbaddr );

int  t6_libusb_FilpJpegFrame(PT6EVDI t6dev,char *jpgimage ,int jpgsize ,int flag);
int  t6_libusb_FilpNV12Frame(PT6EVDI t6dev,unsigned char *nv12image ,int nv12size,int flag);
int  t6_libusb_FilpYV12Frame(PT6EVDI t6dev,unsigned char *yv12image ,int yv12size,int flag);
int  t6_libusb_SendAudio(PT6EVDI t6dev,char * data , int len  );
int  t6_libusb_dongle_reset(PT6EVDI t6dev);
int  t6_libusb_get_monitorstatus(PT6EVDI t6dev); // veiw = 0 hdmi , view =1 vga; 
int t6_libusb_get_monitorstatus2(libusb_device_handle *t6usbdev, int disp_interface); // veiw = 0 hdmi , view =1 vga; 
int  t6_libusb_test(libusb_device_handle* t6usbdev);
int  t6_libusb_get_interrupt(PT6EVDI t6dev,char * data , int len );
int t6_libusb_get_interrupt3(libusb_device_handle *t6usbdev,char * data , int len );

int  t6_libusb_get_touch(libusb_device_handle* t6usbdev);
int  t6_libusb_set_touch(libusb_device_handle* t6usbdev,char on);


int  t6_libusb_get_usagetime(libusb_device_handle* t6usbdev);
int  t6_libusb_set_usagetime(libusb_device_handle* t6usbdev ,int min);

int  t6_libusb_get_brightness(libusb_device_handle* t6usbdev);
int  t6_libusb_set_brightness(libusb_device_handle* t6usbdev ,char bvalue);
	

int  t6_libusb_get_version(libusb_device_handle* t6usbdev ,char index);
int  t6_libusb_get_sn(libusb_device_handle* t6usbdev ,char* sn);

int  t6_libusb_set_rotate(libusb_device_handle* t6usbdev ,char rotate);
int  t6_libusb_set_custom(libusb_device_handle* t6usbdev );
int  t6_write_rom_date(libusb_device_handle* t6usbdev , char *buf ,int len);
void t6_libusb_donglereset2(libusb_device_handle* t6usbdev);
int  t6_libusb_get_interrupt2(libusb_device_handle* t6usbdev,char * data , int len );
int  t6_libusb_set_monitor_power2(libusb_device_handle* t6usbdev,char on);
int  t6_libusb_get_linkspeed(PT6EVDI t6dev);
int  t6_revise_edid(PT6EVDI t6dev, int edid_size, unsigned char *bRun4k30);
int  t6_libusb_get_4K_capbability(PT6EVDI t6dev);
int t6_libusb_set_cursor_shape(libusb_device_handle* t6usbdev, int cur_idx, int disp_no, int w, int h, unsigned char * data , int len);
int t6_libusb_set_cursor_postion(libusb_device_handle* t6usbdev, int x, int y, int disp_no);
int t6_libusb_set_cursor_state(libusb_device_handle* t6usbdev, int cur_idx, int disp_no, int enable);

void ShowRomMsg(libusb_device_handle* t6usbdev);
int  t6_set_cousor_image(libusb_device_handle* t6usbdev,char* src_format  ,int len ,int index);
int t6_set_cousor_onoff(libusb_device_handle* t6usbdev,int on ,int index);
int t6_set_cousor_pos(libusb_device_handle* t6usbdev,int x ,int y);


#endif

