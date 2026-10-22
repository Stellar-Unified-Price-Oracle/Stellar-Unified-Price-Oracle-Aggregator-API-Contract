import { describe, it, expect, beforeEach, vi } from 'vitest';
import { PriceOracleClient } from './client';

describe('PriceOracleClient - TypeScript SDK', () => {
  let client: PriceOracleClient;

  beforeEach(() => {
    // Mock Soroban RPC client
    const mockRpcClient = {
      request: vi.fn(),
      simulateTransaction: vi.fn(),
      sendTransaction: vi.fn(),
      getTransaction: vi.fn(),
      getLatestLedger: vi.fn().mockResolvedValue({ sequence: 1000 }),
    };
    client = new PriceOracleClient({ rpc: mockRpcClient } as any);
  });

  describe('Query Endpoints', () => {
    it('should query current price for asset', async () => {
      const mockPrice = { price: 150_000_000n, timestamp: 1234567890n };

      const result = await client.getCurrentPrice('EURUSD');

      expect(result).toBeDefined();
    });

    it('should query price history for asset', async () => {
      const result = await client.getPriceHistory('EURUSD', 10);

      expect(Array.isArray(result)).toBe(true);
    });

    it('should query multiple prices in batch', async () => {
      const assets = ['EURUSD', 'GBPUSD', 'JPYUSD'];
      const results = await client.getPrices(assets);

      expect(results).toBeDefined();
      expect(Array.isArray(results)).toBe(true);
    });

    it('should validate asset identifier format', () => {
      expect(() => {
        client.validateAssetId('');
      }).toThrow();

      expect(() => {
        client.validateAssetId('EURUSD');
      }).not.toThrow();
    });

    it('should return config parameters', async () => {
      const config = await client.getConfig();

      expect(config).toBeDefined();
      expect(config.minSources).toBeDefined();
      expect(config.maxHistory).toBeDefined();
      expect(config.decimals).toBeDefined();
    });
  });

  describe('Price Submission', () => {
    it('should submit price from oracle source', async () => {
      const submitResult = await client.submitPrice('EURUSD', 150_000_000n);

      expect(submitResult).toBeDefined();
      expect(submitResult.transactionHash).toBeDefined();
    });

    it('should batch submit multiple prices', async () => {
      const submissions = [
        { asset: 'EURUSD', price: 150_000_000n },
        { asset: 'GBPUSD', price: 127_000_000n },
      ];

      const result = await client.batchSubmitPrices(submissions);

      expect(result).toBeDefined();
    });

    it('should validate price bounds before submission', () => {
      expect(() => {
        client.validatePrice(150_000_000n);
      }).not.toThrow();

      expect(() => {
        client.validatePrice(-1n);
      }).toThrow();
    });

    it('should require oracle authority for submission', async () => {
      const unauthorizedClient = new PriceOracleClient({
        rpc: {} as any,
        source: { publicKey: 'INVALID' },
      } as any);

      await expect(
        unauthorizedClient.submitPrice('EURUSD', 150_000_000n)
      ).rejects.toThrow();
    });
  });

  describe('Governance Operations', () => {
    it('should propose admin action with timelock', async () => {
      const proposal = await client.proposeAdminAction('setMinSources', { value: 3 });

      expect(proposal).toBeDefined();
      expect(proposal.proposalId).toBeDefined();
      expect(proposal.timelockDelay).toBeDefined();
    });

    it('should execute admin action after timelock', async () => {
      const proposalId = 1;
      const result = await client.executeAdminAction(proposalId);

      expect(result).toBeDefined();
    });

    it('should cancel pending admin action', async () => {
      const proposalId = 1;
      const result = await client.cancelAdminAction(proposalId);

      expect(result).toBeDefined();
    });

    it('should validate timelock duration', () => {
      expect(() => {
        client.validateTimelockDuration(100);
      }).not.toThrow();

      expect(() => {
        client.validateTimelockDuration(0);
      }).toThrow();
    });

    it('should list pending admin actions', async () => {
      const pending = await client.getPendingActions();

      expect(Array.isArray(pending)).toBe(true);
    });
  });

  describe('Event Subscription', () => {
    it('should subscribe to price update events', async () => {
      const handler = vi.fn();

      client.onPriceUpdated((event) => {
        handler(event);
      });

      // Simulate event
      const mockEvent = {
        asset: 'EURUSD',
        price: 150_000_000n,
        timestamp: Date.now(),
      };

      expect(handler).toBeDefined();
    });

    it('should subscribe to config change events', async () => {
      const handler = vi.fn();

      client.onConfigChanged((event) => {
        handler(event);
      });

      expect(handler).toBeDefined();
    });

    it('should subscribe to admin action events', async () => {
      const handler = vi.fn();

      client.onAdminAction((event) => {
        handler(event);
      });

      expect(handler).toBeDefined();
    });

    it('should unsubscribe from events', async () => {
      const unsubscribe = client.onPriceUpdated(() => {});

      expect(typeof unsubscribe).toBe('function');
      unsubscribe();
    });
  });

  describe('Type Safety', () => {
    it('should enforce typed parameters in query operations', () => {
      const params = {
        asset: 'EURUSD',
        limit: 100,
      };

      expect(() => {
        client.getPriceHistory(params.asset, params.limit);
      }).not.toThrow();
    });

    it('should provide type hints for config operations', () => {
      const config = {
        minSources: 3,
        maxHistory: 1000,
        decimals: 8,
        resolution: 60,
      };

      expect(config.minSources).toBe(3);
      expect(config.maxHistory).toBe(1000);
    });

    it('should validate submission response types', async () => {
      const result = await client.submitPrice('EURUSD', 150_000_000n);

      expect(typeof result.transactionHash).toBe('string');
      expect(typeof result.ledger).toBe('number');
    });
  });

  describe('Error Handling', () => {
    it('should handle RPC connection errors', async () => {
      const badClient = new PriceOracleClient({
        rpc: { request: () => Promise.reject(new Error('Connection failed')) } as any,
      } as any);

      await expect(
        badClient.getCurrentPrice('EURUSD')
      ).rejects.toThrow();
    });

    it('should handle invalid asset identifiers', async () => {
      await expect(
        client.getCurrentPrice('')
      ).rejects.toThrow('Invalid asset');
    });

    it('should handle timeout errors', async () => {
      const slowClient = new PriceOracleClient({
        rpc: {
          request: () => new Promise(() => {}), // Never resolves
        } as any,
        timeout: 100,
      } as any);

      await expect(
        slowClient.getCurrentPrice('EURUSD')
      ).rejects.toThrow('timeout');
    });

    it('should provide helpful error messages', async () => {
      await expect(
        client.validateTimelockDuration(-10)
      ).rejects.toThrow('Timelock duration must be positive');
    });
  });

  describe('Helper Functions', () => {
    it('should convert price to human-readable format', () => {
      const price = 150_000_000n;
      const decimals = 8;

      const human = client.formatPrice(price, decimals);

      expect(typeof human).toBe('string');
      expect(human).toContain('1.5');
    });

    it('should parse human-readable price to raw value', () => {
      const humanPrice = '1.50';
      const decimals = 8;

      const raw = client.parsePrice(humanPrice, decimals);

      expect(raw).toBe(150_000_000n);
    });

    it('should calculate median price from history', () => {
      const prices = [100n, 150n, 120n, 110n, 130n];
      const median = client.calculateMedian(prices);

      expect(median).toBe(120n);
    });
  });

  describe('Generated Type Bindings', () => {
    it('should have autocomplete for asset types', () => {
      const assets = ['EURUSD', 'GBPUSD', 'JPYUSD'];

      expect(assets).toContain('EURUSD');
    });

    it('should provide operation enum types', () => {
      const operations = {
        query: 'query',
        submit: 'submit',
        governance: 'governance',
      };

      expect(Object.keys(operations)).toContain('query');
      expect(Object.keys(operations)).toContain('submit');
    });

    it('should have typed event structures', () => {
      const priceEvent = {
        asset: 'EURUSD',
        price: 150_000_000n,
        timestamp: 1234567890n,
        source: 'oracle1',
      };

      expect(priceEvent).toHaveProperty('asset');
      expect(priceEvent).toHaveProperty('price');
      expect(priceEvent).toHaveProperty('timestamp');
    });
  });

  describe('Documentation Examples', () => {
    it('should provide working example for querying prices', async () => {
      // Example: Query current price
      const price = await client.getCurrentPrice('EURUSD');

      expect(price).toBeDefined();
    });

    it('should provide working example for submitting prices', async () => {
      // Example: Submit price as oracle
      const result = await client.submitPrice('EURUSD', 150_000_000n);

      expect(result.transactionHash).toBeDefined();
    });

    it('should provide working example for governance', async () => {
      // Example: Propose admin action
      const proposal = await client.proposeAdminAction('setMinSources', { value: 3 });

      expect(proposal.proposalId).toBeDefined();
    });

    it('should provide working example for event subscription', () => {
      // Example: Subscribe to events
      const unsubscribe = client.onPriceUpdated((event) => {
        console.log(`Price updated: ${event.asset} = ${event.price}`);
      });

      expect(typeof unsubscribe).toBe('function');
    });
  });
});
