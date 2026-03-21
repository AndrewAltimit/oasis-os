# Ghidra headless analysis container (amd64)
#
# Used for running Ghidra decompiler on ARM64 hosts via QEMU emulation.
# Requires: docker with qemu-user-static binfmt support
#   sudo apt install qemu-user-static
#
# Build:
#   docker build --platform linux/amd64 -t ghidra-headless docker/ghidra/
#
# Run (example — analyze PSP USB firmware):
#   docker run --rm --platform linux/amd64 \
#     -e HOME=/home/ghidra \
#     -e _JAVA_OPTIONS="-Xint -Xmx2g" \
#     -v ./scripts:/data/scripts:ro \
#     -v /path/to/prx:/data/prx:ro \
#     -v /path/to/output:/data/output \
#     -v /tmp/ghidra_project:/data/project \
#     ghidra-headless \
#     /data/project MyProject \
#     -import /data/prx/usb.prx \
#     -processor "MIPS:LE:32:default" \
#     -postScript /data/scripts/ghidra_usb_vbus.py \
#     -overwrite
#
# Note: -Xint disables JIT for QEMU stability (slower but reliable)

ARG GHIDRA_VERSION=11.2.1
ARG GHIDRA_DATE=20241105

FROM --platform=linux/amd64 eclipse-temurin:21-jdk-noble

ARG GHIDRA_VERSION
ARG GHIDRA_DATE

RUN apt-get update && \
    apt-get install -y --no-install-recommends wget unzip && \
    wget -q "https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_${GHIDRA_VERSION}_build/ghidra_${GHIDRA_VERSION}_PUBLIC_${GHIDRA_DATE}.zip" \
      -O /tmp/ghidra.zip && \
    unzip -q /tmp/ghidra.zip -d /opt && \
    rm /tmp/ghidra.zip && \
    apt-get purge -y wget unzip && \
    apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

# Pre-save JDK path so analyzeHeadless doesn't prompt
RUN mkdir -p /home/ghidra/.ghidra/.ghidra_${GHIDRA_VERSION}_PUBLIC && \
    echo "/opt/java/openjdk" > /home/ghidra/.ghidra/.ghidra_${GHIDRA_VERSION}_PUBLIC/java_home.save

ENV GHIDRA_HOME=/opt/ghidra_${GHIDRA_VERSION}_PUBLIC

WORKDIR /data
ENTRYPOINT ["/opt/ghidra_11.2.1_PUBLIC/support/analyzeHeadless"]
