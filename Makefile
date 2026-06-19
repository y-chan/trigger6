trigger6-y := \
	trigger6_commands.o \
	trigger6_connector.o \
	trigger6_drv.o

obj-m := trigger6.o

KVER ?= $(shell uname -r)
KSRC ?= /lib/modules/$(KVER)/build
SPARSE ?= /usr/bin/sparse

all:	modules

modules:
ifeq ($(wildcard $(KSRC)/Makefile),)
	$(error KSRC does not point to a Linux kernel build tree: $(KSRC). Set KSRC=/path/to/linux-headers-or-build)
endif
	"$(MAKE)" CHECK="$(SPARSE)" -C "$(KSRC)" M="$(CURDIR)" modules

clean:
ifeq ($(wildcard $(KSRC)/Makefile),)
	$(error KSRC does not point to a Linux kernel build tree: $(KSRC). Set KSRC=/path/to/linux-headers-or-build)
endif
	"$(MAKE)" -C "$(KSRC)" M="$(CURDIR)" clean
	$(RM) "$(CURDIR)/Module.symvers" "$(CURDIR)"/*.ur-safe
