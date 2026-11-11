# The candidate must run without the build runner, Cargo, Nix or network access.
FROM ubuntu:24.04
ARG UBUNTU_ARCHIVE=http://mirrors.edge.kernel.org/ubuntu
# Match the runner's reachable mirror while retaining signed HTTP bootstrap.
RUN sed -i \
        -e "s|http://archive.ubuntu.com/ubuntu|$UBUNTU_ARCHIVE|g" \
        -e "s|http://security.ubuntu.com/ubuntu|$UBUNTU_ARCHIVE|g" \
        /etc/apt/sources.list.d/ubuntu.sources
RUN for attempt in 1 2 3; do \
        apt-get --error-on=any -o Acquire::Retries=5 update && break; \
        if [ "$attempt" = 3 ]; then echo "apt-get update failed 3 times" >&2; exit 1; fi; \
        sleep $((attempt * 30)); \
    done \
    && apt-get -o Acquire::Retries=5 install --yes --no-install-recommends ca-certificates libstdc++6 python3 \
    && rm -rf /var/lib/apt/lists/*
COPY bundle /bundle
COPY checks /checks
ENV PYTHONDONTWRITEBYTECODE=1
USER 10001:10001
ENTRYPOINT ["python3", "/checks/hosted_pilot_local.py", "--binary", "/bundle/leptoskit-cloud", "--site", "/bundle/site"]
