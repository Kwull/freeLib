import { test, expect } from '@playwright/test';
import { pickAuthor, booksOfAuthor } from './helpers';

test('browse an author, select books, open send dialog', async ({ page, request }) => {
  const author = await pickAuthor(request, 1, 2);
  const books = await booksOfAuthor(request, author.id, 1, 2);
  test.skip(books.length < 2, 'picked author does not have 2 books in the synthetic library');

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Authors' })).toBeVisible({ timeout: 15000 });

  await page.getByLabel('Filter authors').fill(author.name);
  await page.getByRole('button', { name: new RegExp(author.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();

  await expect(page.getByText(books[0].title)).toBeVisible();

  await page.getByRole('checkbox', { name: `Select ${books[0].title}` }).check();
  await page.getByRole('checkbox', { name: `Select ${books[1].title}` }).check();

  await expect(page.getByText('2 selected')).toBeVisible();
  await page.getByRole('button', { name: 'Send to…' }).click();

  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText(`${books[0].title}, ${books[1].title}`)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kindle Send by email EPUB' })).toBeVisible();
});
