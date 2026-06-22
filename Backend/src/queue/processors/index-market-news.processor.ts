import { Processor, Process, OnQueueFailed } from '@nestjs/bull';
import type { Job } from 'bull';
import { Inject, Logger, Optional } from '@nestjs/common';
import { InjectQueue } from '@nestjs/bull';
import type { Queue } from 'bull';
import { JobResult } from '../types/job.types';
import { ValidationError } from '../types/errors';
import { MetricsService } from '../../observability/services/metrics.service';

interface IndexMarketNewsData {
  source: string;
  startDate?: string;
  endDate?: string;
  limit?: number;
}

interface NewsItem {
  id: string;
  title: string;
  content: string;
  source: string;
  publishedAt: string;
  tags: string[];
}

@Processor('index-market-news')
export class IndexMarketNewsProcessor {
  private readonly logger = new Logger(IndexMarketNewsProcessor.name);

  constructor(
    @InjectQueue('failed-jobs') private readonly dlqQueue: Queue,
    @Optional() @Inject(MetricsService) private readonly metrics?: MetricsService,
  ) {}

  @Process()
  async handleIndexMarketNews(
    job: Job<IndexMarketNewsData>,
  ): Promise<JobResult> {
    const { source, startDate, endDate, limit = 100 } = job.data;
    const start = Date.now();

    this.logger.log(
      `Processing index-market-news job ${job.id}: source=${source}, limit=${limit}`,
    );

    this.metrics?.recordJobStart('index-market-news');

    try {
      await job.progress(10);

      if (!source) {
        throw new ValidationError('Missing required field: source');
      }

      this.logger.debug(`Fetching market news from ${source}...`);
      await job.progress(20);

      const newsItems = await this.fetchNews(source, startDate, endDate, limit);
      await job.progress(50);

      const enrichedNews = await this.enrichNews(newsItems);
      await job.progress(75);

      const indexResult = await this.indexNews(enrichedNews);
      await job.progress(100);

      this.logger.log(
        `Market news indexed: ${indexResult.indexedCount} items, ${indexResult.failedCount} failed`,
      );

      const duration = (Date.now() - start) / 1000;
      this.metrics?.recordJobCompleted('index-market-news', duration);

      return {
        success: true,
        data: {
          source,
          indexedCount: indexResult.indexedCount,
          failedCount: indexResult.failedCount,
          lastIndexedAt: new Date().toISOString(),
          itemsProcessed: enrichedNews.length,
        },
      };
    } catch (error) {
      const duration = (Date.now() - start) / 1000;
      this.metrics?.recordJobFailed('index-market-news', duration, error.constructor.name);
      this.logger.error(
        `Failed to index market news: ${error.message}`,
        error.stack,
      );
      throw error;
    }
  }

  @OnQueueFailed()
  async onFailed(job: Job<IndexMarketNewsData>, err: Error): Promise<void> {
    const attemptsExhausted = job.attemptsMade >= (job.opts.attempts ?? 1);
    const isPermanent = (err as any).retryable === false;

    if (attemptsExhausted || isPermanent) {
      this.logger.error(
        `Job ${job.id} exhausted retries or is permanent — routing to DLQ`,
        err.stack,
      );
      await this.dlqQueue.add({
        originalQueue: 'index-market-news',
        originalJobId: job.id,
        failedReason: err.message,
        payload: job.data,
      });
    }
  }

  private async fetchNews(
    source: string,
    startDate?: string,
    endDate?: string,
    limit?: number,
  ): Promise<NewsItem[]> {
    return new Promise((resolve) => {
      setTimeout(() => {
        const newsItems: NewsItem[] = [];
        for (let i = 0; i < (limit || 10); i++) {
          newsItems.push({
            id: `news-${Date.now()}-${i}`,
            title: `Market News ${i + 1} from ${source}`,
            content: `This is a sample news article about market trends and opportunities from ${source}.`,
            source,
            publishedAt: new Date(
              Date.now() - Math.random() * 86400000,
            ).toISOString(),
            tags: ['market', 'crypto', source.toLowerCase()],
          });
        }
        resolve(newsItems);
      }, 1500);
    });
  }

  private async enrichNews(newsItems: NewsItem[]): Promise<NewsItem[]> {
    return new Promise((resolve) => {
      setTimeout(() => {
        const enriched = newsItems.map((item) => ({
          ...item,
          tags: [...item.tags, 'enriched', 'indexed'],
        }));
        resolve(enriched);
      }, 1000);
    });
  }

  private async indexNews(
    newsItems: NewsItem[],
  ): Promise<{ indexedCount: number; failedCount: number }> {
    return new Promise((resolve) => {
      setTimeout(() => {
        const failedCount = Math.floor(newsItems.length * 0.05);
        const indexedCount = newsItems.length - failedCount;
        resolve({ indexedCount, failedCount });
      }, 1500);
    });
  }
}
