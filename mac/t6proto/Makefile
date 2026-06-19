CC ?= cc
CFLAGS ?= -std=c11 -Wall -Wextra -Werror -pedantic -O2
CPPFLAGS ?= -Iinclude

BUILD_DIR := build
LIB := $(BUILD_DIR)/libt6proto.a
TEST := $(BUILD_DIR)/t6proto_tests

SRC := src/t6proto.c
OBJ := $(BUILD_DIR)/t6proto.o

.PHONY: all test clean

all: $(LIB)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(OBJ): $(SRC) include/t6proto.h | $(BUILD_DIR)
	$(CC) $(CPPFLAGS) $(CFLAGS) -c $< -o $@

$(LIB): $(OBJ)
	ar rcs $@ $<

$(TEST): tests/t6proto_tests.c $(LIB) include/t6proto.h | $(BUILD_DIR)
	$(CC) $(CPPFLAGS) $(CFLAGS) tests/t6proto_tests.c $(LIB) -o $@

test: $(TEST)
	$(TEST)

clean:
	rm -rf $(BUILD_DIR)

