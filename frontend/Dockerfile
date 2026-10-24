# syntax=docker/dockerfile:1

# ---- build ----
FROM node:22-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY . .
RUN npm run build

# ---- runtime ----
FROM nginxinc/nginx-unprivileged:1.27-alpine

# Defaults for local/compose use; Helm sets API_UPSTREAM to http://<release>-api:8080.
# For GitHub Enterprise Server, set CSP_IMG_SRC to the GHES avatar origin (e.g. https://ghe.example.com).
ENV API_UPSTREAM=http://api:8080 \
    CSP_IMG_SRC=https://avatars.githubusercontent.com

COPY --from=build /app/dist /usr/share/nginx/html
COPY nginx/default.conf.template /etc/nginx/templates/default.conf.template

USER 101
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
