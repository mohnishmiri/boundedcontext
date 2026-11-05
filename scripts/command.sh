cd "C:\GITHUB\ATTCC\DP1\AIFC\bounded-context-ui\scripts"
scripts/push-images.sh 1.0.4

cd "C:\GITHUB\ATTCC\DP1\AIFC\bounded-context-ui\deploy\helm\bounded-context-portal"
CHART=. 
NS=bounded-context
TAG=1.0.4
IMG="--set api.image.tag=$TAG --set worker.image.tag=$TAG --set frontend.image.tag=$TAG"

helm lint $CHART -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG
helm template bcp $CHART -n $NS -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG > /tmp/bcp-perf.yaml
helm upgrade --install bcp $CHART -n $NS -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG --dry-run=server
helm upgrade --install bcp $CHART -n $NS -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG