GREEN   = \033[1;32m

WHITE   = \033[0;m

NAME    = T6evdi
NAME2   = T6contrl

CC      = gcc -g

RM      = rm -f

SRCS    = main.c t6usbdongle.c cea_edid.c simclist.c queue.c
SRCS2   = T6contrl.c t6usbdongle.c cea_edid.c

OBJS    = $(SRCS:.c=.o)
OBJS2   = $(SRCS2:.c=.o)

CFLAGS  =

LDFLAGS = -levdi -lusb-1.0 -lturbojpeg -lc -lm -lpthread

all: $(NAME) $(NAME2) 

$(NAME): $(OBJS)
	@$(CC) $(OBJS) -o $(NAME) $(LDFLAGS)
	@printf "\n[$(GREEN)OK$(WHITE)] Binary : $(NAME)\n"
	@echo "-------------------\n"

$(NAME2): $(OBJS2)
	@$(CC) $(OBJS2) -o $(NAME2) $(LDFLAGS)
	@printf "\n[$(GREEN)OK$(WHITE)] Binary : $(NAME2)\n"
	@echo "-------------------\n"

%.o :    %.c
	@$(CC) $(CFLAGS) -c -o $@ $<
	@printf "[$(GREEN)OK$(WHITE)] $<\n"


fclean:
	$(RM) $(OBJS) $(OBJS2)

clean: fclean
	$(RM) $(NAME) $(NAME2)
