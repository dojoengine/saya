# Directory where the compiled classes will be stored
BUILD_DIR := ./contracts/build

$(BUILD_DIR)/snos.json: ./cairo-lang | $(BUILD_DIR)
	cairo-compile $</src/starkware/starknet/core/os/os.cairo --output $@ --cairo_path $</src

$(BUILD_DIR):
	mkdir -p $@
