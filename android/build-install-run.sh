#!/usr/bin/env bash
# HandControl Android - Build, Install, and Run Script
# This script builds the debug APK, installs it to a connected device, and runs it.

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}=== HandControl Android Build & Install ===${NC}"

# Check if ADB device is connected
echo -e "\n${YELLOW}Checking for connected devices...${NC}"
DEVICE_COUNT=$(adb devices | grep -v "List of devices" | grep "device$" | wc -l)

if [ "$DEVICE_COUNT" -eq 0 ]; then
    echo -e "${RED}Error: No Android devices connected${NC}"
    echo "Please connect a device or start an emulator and try again."
    exit 1
fi

echo -e "${GREEN}Found $DEVICE_COUNT device(s) connected${NC}"

# Build the APK
echo -e "\n${YELLOW}Building debug APK...${NC}"
gradle assembleDebug

if [ $? -eq 0 ]; then
    echo -e "${GREEN}Build successful!${NC}"
else
    echo -e "${RED}Build failed!${NC}"
    exit 1
fi

# Install the APK
echo -e "\n${YELLOW}Installing APK to device...${NC}"
adb install -r app/build/outputs/apk/debug/app-debug.apk

if [ $? -eq 0 ]; then
    echo -e "${GREEN}Installation successful!${NC}"
else
    echo -e "${RED}Installation failed!${NC}"
    exit 1
fi

# Launch the app
echo -e "\n${YELLOW}Launching HandControl app...${NC}"
adb shell am start -n com.handcontrol/.MainActivity

if [ $? -eq 0 ]; then
    echo -e "${GREEN}App launched successfully!${NC}"
    echo -e "\n${GREEN}=== All steps completed successfully ===${NC}"
else
    echo -e "${RED}Failed to launch app!${NC}"
    exit 1
fi
