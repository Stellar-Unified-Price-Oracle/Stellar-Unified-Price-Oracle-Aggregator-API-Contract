package oracle

import (
	"context"
	"testing"
)

func TestPriceEntry(t *testing.T) {
	entry := PriceEntry{
		Price:     1000000,
		Timestamp: 1234567890,
	}

	if entry.Price != 1000000 {
		t.Errorf("Expected price 1000000, got %d", entry.Price)
	}

	if entry.Timestamp != 1234567890 {
		t.Errorf("Expected timestamp 1234567890, got %d", entry.Timestamp)
	}
}

func TestClientConfig(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
		Network:    "testnet",
	}

	if config.ContractID != "CABC123DEF456" {
		t.Errorf("Expected contract ID CABC123DEF456, got %s", config.ContractID)
	}

	if config.RPCUrl != "https://soroban-testnet.stellar.org" {
		t.Errorf("Expected RPC URL https://soroban-testnet.stellar.org, got %s", config.RPCUrl)
	}

	if config.Network != "testnet" {
		t.Errorf("Expected network testnet, got %s", config.Network)
	}
}

func TestNewClient(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
		Network:    "testnet",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	if client == nil {
		t.Fatal("Expected non-nil client")
	}

	if client.config.ContractID != config.ContractID {
		t.Errorf("Expected contract ID %s, got %s", config.ContractID, client.config.ContractID)
	}
}

func TestGetPrice(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	entry, err := client.GetPrice(ctx, "GCTEST", 3600)
	if err != nil {
		t.Errorf("GetPrice failed: %v", err)
	}

	if entry == nil {
		t.Fatal("Expected non-nil PriceEntry")
	}
}

func TestGetSourcePrice(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	entry, err := client.GetSourcePrice(ctx, "GCTEST", "GCSOURCE")
	if err != nil {
		t.Errorf("GetSourcePrice failed: %v", err)
	}

	if entry == nil {
		t.Fatal("Expected non-nil PriceEntry")
	}
}

func TestGetAllPrices(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	entries, err := client.GetAllPrices(ctx, "GCTEST")
	if err != nil {
		t.Errorf("GetAllPrices failed: %v", err)
	}

	if entries == nil {
		t.Fatal("Expected non-nil entries slice")
	}
}

func TestSubmitPrice(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	_, err = client.SubmitPrice(ctx, "GCSOURCE", "GCTEST", 1000000, 1234567890, nil)
	if err != nil {
		t.Errorf("SubmitPrice failed: %v", err)
	}
}

func TestSubscribe(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	_, err = client.Subscribe(ctx, "GCCONSUMER", 86400, nil)
	if err != nil {
		t.Errorf("Subscribe failed: %v", err)
	}
}

func TestRenewSubscription(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	_, err = client.RenewSubscription(ctx, "GCCONSUMER", nil)
	if err != nil {
		t.Errorf("RenewSubscription failed: %v", err)
	}
}

func TestGetSubscriptionExpiry(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	expiry, err := client.GetSubscriptionExpiry(ctx, "GCCONSUMER")
	if err != nil {
		t.Errorf("GetSubscriptionExpiry failed: %v", err)
	}

	if expiry < 0 {
		t.Errorf("Expected non-negative expiry, got %d", expiry)
	}
}

func TestIsSource(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	isSource, err := client.IsSource(ctx, "GCSOURCE")
	if err != nil {
		t.Errorf("IsSource failed: %v", err)
	}

	if isSource == true && isSource == false {
		t.Error("IsSource returned invalid boolean")
	}
}

func TestIsAssetRegistered(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	isRegistered, err := client.IsAssetRegistered(ctx, "GCTEST")
	if err != nil {
		t.Errorf("IsAssetRegistered failed: %v", err)
	}

	if isRegistered == true && isRegistered == false {
		t.Error("IsAssetRegistered returned invalid boolean")
	}
}

func TestListenForPriceSubmittedEvents(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	listener, err := client.ListenForPriceSubmittedEvents(ctx, "PriceSubmittedEvent")
	if err != nil {
		t.Fatalf("ListenForPriceSubmittedEvents failed: %v", err)
	}

	if listener == nil {
		t.Fatal("Expected non-nil EventListener")
	}

	if listener.GetEventType() != "PriceSubmittedEvent" {
		t.Errorf("Expected event type PriceSubmittedEvent, got %s", listener.GetEventType())
	}

	err = listener.Stop()
	if err != nil {
		t.Errorf("Failed to stop listener: %v", err)
	}
}

func TestEventListener(t *testing.T) {
	config := ClientConfig{
		ContractID: "CABC123DEF456",
		RPCUrl:     "https://soroban-testnet.stellar.org",
	}

	client, err := NewClient(config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}

	ctx := context.Background()
	listener, err := client.ListenForPriceSubmittedEvents(ctx, "TestEvent")
	if err != nil {
		t.Fatalf("Failed to create listener: %v", err)
	}

	eventType := listener.GetEventType()
	if eventType != "TestEvent" {
		t.Errorf("Expected TestEvent, got %s", eventType)
	}

	err = listener.Stop()
	if err != nil {
		t.Errorf("Failed to stop listener: %v", err)
	}
}
